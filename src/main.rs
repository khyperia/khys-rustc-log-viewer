use bumpalo::Bump;
use eframe::{
    CreationContext,
    egui::{
        CentralPanel, Color32, ComboBox, FontId, Frame, InputState, Key, Modifiers, Panel,
        TextFormat, Ui, UiBuilder, text::LayoutJob,
    },
};
use std::{
    collections::HashMap,
    error::Error,
    fmt::{self, Debug, Display, Formatter},
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
    str::{Chars, FromStr},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SendError, TryRecvError},
    },
    time::Instant,
};
use yoke::Yoke;

#[allow(dead_code)]
mod alacritty {
    use eframe::egui::Color32;

    // colors copied from (and inspired by/derived from) alacritty's default scheme:
    // https://github.com/alacritty/alacritty/blob/f99dc71708d31d5c32d4b3fa611f9a87bf22657e/alacritty/src/config/color.rs#L204-L211
    pub const BLACK: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
    pub const RED: Color32 = Color32::from_rgb(0xac, 0x42, 0x42);
    pub const GREEN: Color32 = Color32::from_rgb(0x90, 0xa9, 0x59);
    pub const YELLOW: Color32 = Color32::from_rgb(0xf4, 0xbf, 0x75);
    pub const BLUE: Color32 = Color32::from_rgb(0x6a, 0x9f, 0xb5);
    pub const MAGENTA: Color32 = Color32::from_rgb(0xaa, 0x75, 0x9f);
    pub const CYAN: Color32 = Color32::from_rgb(0x75, 0xb5, 0xaa);
    pub const WHITE: Color32 = Color32::from_rgb(0xd8, 0xd8, 0xd8);

    pub const BRIGHT_BLACK: Color32 = Color32::from_rgb(0x6b, 0x6b, 0x6b);
    pub const BRIGHT_RED: Color32 = Color32::from_rgb(0xc5, 0x55, 0x55);
    pub const BRIGHT_GREEN: Color32 = Color32::from_rgb(0xaa, 0xc4, 0x74);
    pub const BRIGHT_YELLOW: Color32 = Color32::from_rgb(0xfe, 0xca, 0x88);
    pub const BRIGHT_BLUE: Color32 = Color32::from_rgb(0x82, 0xb8, 0xc8);
    pub const BRIGHT_MAGENTA: Color32 = Color32::from_rgb(0xc2, 0x8c, 0xb8);
    pub const BRIGHT_CYAN: Color32 = Color32::from_rgb(0x93, 0xd3, 0xc3);
    pub const BRIGHT_WHITE: Color32 = Color32::from_rgb(0xf8, 0xf8, 0xf8);

    pub const DIM_BLACK: Color32 = Color32::from_rgb(0x0f, 0x0f, 0x0f);
    pub const DIM_RED: Color32 = Color32::from_rgb(0x71, 0x2b, 0x2b);
    pub const DIM_GREEN: Color32 = Color32::from_rgb(0x5f, 0x6f, 0x3a);
    pub const DIM_YELLOW: Color32 = Color32::from_rgb(0xa1, 0x7e, 0x4d);
    pub const DIM_BLUE: Color32 = Color32::from_rgb(0x45, 0x68, 0x77);
    pub const DIM_MAGENTA: Color32 = Color32::from_rgb(0x70, 0x4d, 0x68);
    pub const DIM_CYAN: Color32 = Color32::from_rgb(0x4d, 0x77, 0x70);
    pub const DIM_WHITE: Color32 = Color32::from_rgb(0x8e, 0x8e, 0x8e);
}

const HLSEARCH: Color32 = Color32::from_rgb(0, 0x5c, 0x80);

const SPAN: Color32 = alacritty::YELLOW;
const SPAN_KEYS: Color32 = alacritty::DIM_WHITE;
const FIELD_KEYS: Color32 = alacritty::DIM_WHITE;
const FIELD_VALUES: Color32 = alacritty::WHITE;
const TARGET: Color32 = alacritty::CYAN;
const COLON_COLON: Color32 = alacritty::WHITE;
const SPAN_NAME: Color32 = alacritty::GREEN;
const TIMESTAMP: Color32 = alacritty::DIM_WHITE;
const FILENAME: Color32 = alacritty::DIM_WHITE;

fn log_level_color(level: &str) -> Color32 {
    match level {
        "TRACE" => alacritty::MAGENTA,
        "DEBUG" => alacritty::BLUE,
        "INFO" => alacritty::GREEN,
        "WARN" => alacritty::YELLOW, // TODO: is WARN the right name for this? (i've never seen it)
        _ => alacritty::RED,
    }
}

const LOGPARSE_ERROR: Color32 = alacritty::RED;
fn logparse_color(kind: logparse::SpanKind) -> Color32 {
    match kind {
        logparse::SpanKind::Delimiter(_) => alacritty::WHITE,
        logparse::SpanKind::Separator => alacritty::DIM_WHITE,
        logparse::SpanKind::Number => alacritty::CYAN,
        logparse::SpanKind::Literal => alacritty::CYAN,
        logparse::SpanKind::Lifetime => alacritty::CYAN,
        logparse::SpanKind::String => alacritty::YELLOW,
        logparse::SpanKind::Path => alacritty::DIM_WHITE,
        logparse::SpanKind::Space(_) => alacritty::WHITE,
        logparse::SpanKind::Constructor => alacritty::BLUE,
        logparse::SpanKind::Surroundings => alacritty::DIM_CYAN,
        logparse::SpanKind::Text => alacritty::WHITE,
    }
}

const INDENT: f32 = 8.0;

const LOGPARSE_CONFIG: logparse::Config = logparse::Config {
    collapse_space: false,
};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = if let Some(first) = args.next() {
        if args.next().is_none() {
            PathBuf::from(first)
        } else {
            println!(
                "Only one argument is allowed (the path that was given to RUSTC_LOG_OUTPUT_TARGET)"
            );
            return Ok(());
        }
    } else {
        if let Ok(targ) = std::env::var("RUSTC_LOG_OUTPUT_TARGET") {
            PathBuf::from(targ)
        } else {
            println!("No file provided. Pass the file that was given to RUSTC_LOG_OUTPUT_TARGET");
            return Ok(());
        }
    };
    let mut bump = Bump::new();
    std::thread::scope(|scope| {
        eframe::run_native(
            "Khy's rustc log viewer",
            Default::default(),
            Box::new(|cc| Ok(Box::new(App::new(cc, &mut bump, scope, path)))),
        )
    })?;
    Ok(())
}

struct App<'b> {
    messages: Vec<Message<'b>>,
    messages_reader: Receiver<Message<'b>>,
    ui_has_been_notified: Arc<AtomicBool>,
    start_time: Instant,
    end_time: Option<Instant>,
    scroll_value: ScrollValue,
    state: AppState,
}

#[derive(Default)]
struct AppState {
    entering_search_text: bool,
    search: String,
    search_onscreen: bool,
    timestamps: bool,
    log_levels: bool,
    targets: bool,
    filters: Vec<Filter>,
}

impl AppState {
    fn new() -> Self {
        Self {
            log_levels: true,
            targets: true,
            ..Default::default()
        }
    }

    fn matches_search(&self, s: &str) -> bool {
        if self.search.is_empty() {
            false
        } else {
            s.contains(&self.search)
        }
    }

    fn vdict_matches_search(&self, list: &[HashMap<&str, JsonValue>]) -> bool {
        if self.search.is_empty() {
            false
        } else {
            list.iter()
                .any(|map| dict_matches_search(map, &self.search))
        }
    }
}

impl<'b> App<'b> {
    fn new<'scope, 'env>(
        cc: &CreationContext,
        bump: &'b mut Bump,
        scope: &'scope std::thread::Scope<'scope, 'env>,
        path: PathBuf,
    ) -> Self
    where
        'b: 'scope,
    {
        let ui_has_been_notified = Arc::new(AtomicBool::new(false));
        let atomic_bool = ui_has_been_notified.clone();
        let egui_ctx = cc.egui_ctx.clone();
        let messages_reader = read_lines(bump, scope, path, move || {
            if !atomic_bool.swap(true, Ordering::Relaxed) {
                egui_ctx.request_repaint();
            }
        });
        App {
            messages: vec![],
            messages_reader,
            ui_has_been_notified,
            start_time: Instant::now(),
            end_time: None,
            scroll_value: Default::default(),
            state: AppState::new(),
        }
    }

    // silly nit: index is *inclusive* here
    fn next_search(&self, index: usize) -> Option<usize> {
        (index..self.messages.len())
            .chain(0..index)
            .find(|&index| self.next_search_raw(index))
    }

    // silly nit: index is *exclusive* here
    fn prev_search(&self, index: usize) -> Option<usize> {
        (index..self.messages.len())
            .chain(0..index)
            .rev()
            .find(|&index| self.next_search_raw(index))
    }

    fn next_search_raw(&self, index: usize) -> bool {
        self.messages[index].is_displayed(&self.messages, &self.state)
            && self.messages[index].matches_search(&self.state.search)
    }
}

impl<'b> eframe::App for App<'b> {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.ui_has_been_notified.store(false, Ordering::Relaxed);
        loop {
            match self.messages_reader.try_recv() {
                Ok(message) => self.messages.push(message),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.end_time.is_none() {
                        self.end_time = Some(Instant::now());
                    }
                    break;
                }
            }
        }

        let mut opened_search = false;
        ui.input_mut(|input| {
            scroller_mouse_input(input, &mut self.scroll_value);
            if self.state.entering_search_text {
                if input.consume_key(Modifiers::NONE, Key::Escape) {
                    self.state.entering_search_text = false;
                }
            } else {
                scroller_key_input(
                    input,
                    &mut self.scroll_value,
                    self.messages.len(),
                    ui.clip_rect().height(),
                );
                if input.consume_key(Modifiers::NONE, Key::Slash) {
                    self.state.entering_search_text = true;
                    self.state.search.clear();
                    opened_search = true;
                }
                if !self.state.search.is_empty() {
                    if input.consume_key(Modifiers::NONE, Key::Escape) {
                        self.state.search = String::new();
                    }
                    if input.consume_key(Modifiers::SHIFT, Key::N)
                        && let Some(index) = self.prev_search(self.scroll_value.index)
                    {
                        self.scroll_value.index = index;
                        self.scroll_value.pixel_offset = 0.0;
                    }
                    if input.consume_key(Modifiers::NONE, Key::N)
                        && let Some(index) =
                            self.next_search((self.scroll_value.index + 1) % self.messages.len())
                    {
                        self.scroll_value.index = index;
                        self.scroll_value.pixel_offset = 0.0;
                    }
                }
            }
        });
        let mut ensure_search_onscreen = false;
        if self.state.entering_search_text || !self.state.search.is_empty() {
            Panel::bottom("search panel").show_inside(ui, |ui| {
                if self.state.entering_search_text {
                    ui.horizontal(|ui| {
                        ui.label("/");
                        let rsp = ui.text_edit_singleline(&mut self.state.search);
                        if opened_search {
                            rsp.request_focus();
                        }
                        if rsp.lost_focus() {
                            self.state.entering_search_text = false;
                        }
                        if rsp.changed() {
                            ensure_search_onscreen = true;
                        }
                    });
                } else {
                    ui.label(format!("/{}", self.state.search));
                }
            });
        }
        Panel::top("filter panel").show_inside(ui, |ui| {
            let mut id_salt = 0;
            ui.horizontal(|ui| {
                let end = self.end_time.unwrap_or_else(|| Instant::now());
                ui.label(format!(
                    "{} messages in {:?}",
                    self.messages.len(),
                    end - self.start_time,
                ));
                ui.checkbox(&mut self.state.timestamps, "timestamps");
                ui.checkbox(&mut self.state.log_levels, "log levels");
                ui.checkbox(&mut self.state.targets, "targets");
            });
            self.state.filters.retain_mut(|q| {
                let salt = id_salt;
                id_salt += 1;
                q.ui(salt, ui)
            });
            if ui.button("add filter").clicked() {
                self.state.filters.push(Filter::new())
            }
            ui.add_space(2.0);
        });
        self.state.search_onscreen = false;
        let panel = CentralPanel::default().frame(Frame::new().fill(Color32::from_rgb(38, 50, 56)));
        panel.show_inside(ui, |ui| {
            big_scroller(
                ui,
                &mut self.scroll_value,
                self.messages.len(),
                |ui, index| {
                    if self.messages[index].is_displayed(&self.messages, &self.state) {
                        if let Some(parent) = self.messages[index].parent {
                            let [message, parent] =
                                self.messages.get_disjoint_mut([index, parent]).unwrap();
                            message.ui_outer(Some(parent), &mut self.state, ui);
                        } else {
                            self.messages[index].ui_outer(None, &mut self.state, ui);
                        }
                    }
                },
            );
        });
        if !self.state.search.is_empty()
            && !self.state.search_onscreen
            && ensure_search_onscreen
            && let Some(index) = self.next_search(self.scroll_value.index)
        {
            self.scroll_value.index = index;
            self.scroll_value.pixel_offset = 0.0;
        }
    }
}

#[derive(Default)]
struct ScrollValue {
    index: usize,
    pixel_offset: f32,
}

fn scroller_mouse_input(input: &mut InputState, value: &mut ScrollValue) {
    let scroll_mult = -4.0; // inverted?
    value.pixel_offset += input.smooth_scroll_delta.y * scroll_mult;
    input.smooth_scroll_delta.y = 0.0;
}

fn scroller_key_input(input: &mut InputState, value: &mut ScrollValue, len: usize, ui_height: f32) {
    if input.consume_key(Modifiers::NONE, Key::J) {
        value.pixel_offset += 20.0; // TODO: how to get line height?
    }
    if input.consume_key(Modifiers::NONE, Key::K) {
        value.pixel_offset -= 20.0; // TODO: how to get line height?
    }
    if input.consume_key(Modifiers::NONE, Key::D)
        || input.consume_key(Modifiers::CTRL, Key::D)
        || input.consume_key(Modifiers::NONE, Key::PageDown)
    {
        value.pixel_offset += ui_height / 2.0;
    }
    if input.consume_key(Modifiers::NONE, Key::U)
        || input.consume_key(Modifiers::CTRL, Key::U)
        || input.consume_key(Modifiers::NONE, Key::PageUp)
    {
        value.pixel_offset -= ui_height / 2.0;
    }
    if input.consume_key(Modifiers::NONE, Key::End) || input.consume_key(Modifiers::SHIFT, Key::G) {
        value.index = len - 1;
    }
    if input.consume_key(Modifiers::NONE, Key::Home) || input.consume_key(Modifiers::NONE, Key::G) {
        value.index = 0;
    }
}

fn big_scroller(
    ui: &mut Ui,
    value: &mut ScrollValue,
    len: usize,
    mut draw: impl FnMut(&mut Ui, usize),
) {
    ui.scope(|ui| {
        let max_rect = ui.max_rect();
        let absolute_begin = ui.next_widget_position().y;
        ui.add_space(-value.pixel_offset);
        ui.skip_ahead_auto_ids(value.index);
        for index in value.index..len {
            let begin = ui.next_widget_position().y;
            draw(ui, index);
            let end = ui.next_widget_position().y;
            if end > max_rect.bottom() {
                break;
            }
            if end < absolute_begin && value.index + 1 < len {
                value.index += 1;
                let size = end - begin;
                value.pixel_offset -= size;
                // println!("next scroll: {} {}", value.index, value.pixel_offset);
            }
        }
    });

    // this messes up the drawing of the main content, idk why, so put it after
    if value.pixel_offset < 0.0 && value.index > 0 {
        ui.scope_builder(UiBuilder::new().sizing_pass().invisible(), |ui| {
            while value.pixel_offset < 0.0 && value.index > 0 {
                let begin = ui.next_widget_position().y;
                draw(ui, value.index - 1);
                let end = ui.next_widget_position().y;
                value.index -= 1;
                value.pixel_offset += end - begin;
            }
        });
    }
}

struct Filter {
    kind: FilterKind,
    filter: String,
    exclude: bool,
}

impl Filter {
    fn new() -> Self {
        Self {
            kind: FilterKind::Target,
            filter: String::new(),
            exclude: true,
        }
    }

    fn matches(&self, message: &Message) -> bool {
        if self.filter.is_empty() {
            true
        } else {
            let raw = self.matches_raw(message);
            // TODO: xor :3
            if self.exclude { !raw } else { raw }
        }
    }

    fn matches_raw(&self, message: &Message) -> bool {
        self.kind.run(message, |v| v.contains(&self.filter))
    }

    fn ui(&mut self, id_salt: impl std::hash::Hash, ui: &mut Ui) -> bool {
        ui.horizontal(|ui| {
            ComboBox::from_id_salt(id_salt)
                .selected_text(self.kind.name())
                .show_ui(ui, |ui| {
                    for kind in FilterKind::ALL {
                        ui.selectable_value(&mut self.kind, kind, kind.name());
                    }
                });
            ui.text_edit_singleline(&mut self.filter);
            ui.checkbox(&mut self.exclude, "exclude");
            !ui.button("delete").clicked()
        })
        .inner
    }
}

#[derive(Eq, PartialEq, Clone, Copy)]
enum FilterKind {
    Timestamp,
    Level,
    Fields,
    Target,
    Filename,
    // LineNumber,
    SpanName,
    Spans,
}

impl FilterKind {
    fn run(&self, message: &Message, mut visit: impl FnMut(&str) -> bool) -> bool {
        match self {
            Self::Timestamp => visit(&message.parsed().timestamp),
            Self::Level => visit(&message.parsed().level),
            Self::Fields => message.parsed().fields.iter().any(|(k, v)| {
                visit(k);
                if let JsonValue::String(v) = v {
                    visit(v)
                } else {
                    false
                }
            }),
            Self::Target => visit(&message.parsed().target),
            Self::Filename => visit(&message.parsed().filename),
            Self::SpanName => message.parsed().span.get("name").is_some_and(|n| {
                if let JsonValue::String(n) = n {
                    visit(n)
                } else {
                    false
                }
            }),
            Self::Spans => message.parsed().spans.iter().any(|n| {
                n.iter().any(|(k, v)| {
                    visit(k);
                    if let JsonValue::String(v) = v {
                        visit(v)
                    } else {
                        false
                    }
                })
            }),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Timestamp => "timestamp",
            Self::Level => "level",
            Self::Fields => "fields",
            Self::Target => "target",
            Self::Filename => "filename",
            Self::SpanName => "span name",
            Self::Spans => "spans",
        }
    }

    const ALL: [Self; 7] = [
        Self::Timestamp,
        Self::Level,
        Self::Fields,
        Self::Target,
        Self::Filename,
        Self::SpanName,
        Self::Spans,
    ];
}

fn read_lines<'b: 'scope, 'scope, 'env>(
    bump: &'b mut Bump,
    scope: &'scope std::thread::Scope<'scope, 'env>,
    path: PathBuf,
    notify_ui_thread: impl Fn() + Send + 'static,
) -> Receiver<Message<'b>> {
    let (send, recv) = mpsc::channel();

    scope.spawn(move || {
        let file = File::open(path).unwrap();
        let mut reader = BufReader::new(file);
        let mut parent_stack = vec![];
        let mut i = 0;
        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line).unwrap();
            if bytes_read == 0 {
                break;
            }
            let message = Message::new(bump, &line, i, &mut parent_stack).unwrap();
            match send.send(message) {
                Ok(()) => (),
                Err(SendError(_)) => break,
            }
            notify_ui_thread();
            i += 1;
        }
    });

    recv
}

#[inline]
fn text_format() -> TextFormat {
    TextFormat {
        font_id: FontId::monospace(14.0),
        ..Default::default()
    }
}

#[inline]
fn text_format_color(color: Color32) -> TextFormat {
    TextFormat {
        font_id: FontId::monospace(14.0),
        color,
        ..text_format()
    }
}

struct Message<'b> {
    parsed: ParsedMessage<'b>,
    parent: Option<usize>,
    indent: usize,
    state: MessageState,
}

#[derive(Default)]
struct MessageState {
    hide_children: bool,
    display_filename: bool,
    display_spans: bool,
}

impl<'b> Message<'b> {
    fn new(
        bump: &'b Bump,
        line: &str,
        index: usize,
        parent_stack: &mut Vec<usize>,
    ) -> anyhow::Result<Self> {
        let parsed = custom_parse(bump, line)?;
        let msg = parsed.hop_message();
        let parent = parent_stack.last().cloned();
        if msg == Some(HopMessageKind::Exit) {
            parent_stack.pop();
        }
        let self_indent = parent_stack.len();
        if msg == Some(HopMessageKind::Enter) {
            parent_stack.push(index);
        }

        Ok(Self {
            parsed,
            parent,
            indent: self_indent,
            state: Default::default(),
        })
    }

    fn parsed(&self) -> &ParsedMessage<'b> {
        &self.parsed
    }

    fn is_displayed(&self, messages: &[Message], app_state: &AppState) -> bool {
        // if self.state.hide_self {
        //     return false;
        // }
        for filter in &app_state.filters {
            if !filter.matches(self) {
                return false;
            }
        }
        if let Some(parent) = self.parent {
            let parent = &messages[parent];
            if parent.state.hide_children {
                return false;
            }
            parent.is_displayed(messages, app_state)
        } else {
            true
        }
    }

    fn ui_outer(&mut self, parent: Option<&mut Message>, app_state: &mut AppState, ui: &mut Ui) {
        let mut child_rect = ui.available_rect_before_wrap();
        child_rect.min.x += INDENT * self.indent as f32;
        ui.scope_builder(UiBuilder::new().max_rect(child_rect), |ui| {
            self.ui(parent, app_state, ui);
        });
    }

    fn ui(&mut self, parent: Option<&mut Message>, app_state: &mut AppState, ui: &mut Ui) {
        self.logparse_single_message();
        let mut job = StrBuilder {
            job: LayoutJob::default(),
            app_state,
            found_search: false,
        };
        let parsed = self.parsed();
        self.main_text(&mut job);
        if self.state.display_filename
            || !job.found_search && job.app_state.matches_search(&parsed.filename)
        {
            self.filename(&mut job);
        }
        if self.state.display_spans
            || !job.found_search && job.app_state.vdict_matches_search(&parsed.spans)
        {
            self.spans(&mut job);
        }
        let rsp = if !job.found_search
            && !job.app_state.search.is_empty()
            && self.matches_search(&job.app_state.search)
        {
            job.found_search = true;
            // fallback to highlight the whole message if we're not displaying the matching text
            Frame::NONE
                .fill(HLSEARCH)
                .show(ui, |ui| ui.label(job.job))
                .inner
        } else {
            ui.label(job.job)
        };
        if job.found_search && ui.clip_rect().intersects(rsp.rect) {
            app_state.search_onscreen = true;
        }
        rsp.context_menu(|ui| {
            if let Some(HopMessageKind::Enter) = self.parsed().hop_message() {
                ui.checkbox(&mut self.state.hide_children, "hide children");
            } else if let Some(parent) = parent {
                ui.checkbox(&mut parent.state.hide_children, "hide siblings");
            }
            ui.checkbox(&mut self.state.display_filename, "filename");
            if !self.parsed().spans.is_empty() {
                ui.checkbox(&mut self.state.display_spans, "spans");
            }
            if ui.button("exclude target filter").clicked() {
                app_state.filters.push(Filter {
                    kind: FilterKind::Target,
                    filter: self.parsed().target.to_string(),
                    exclude: true,
                });
            }
        });
    }

    fn logparse_single_message(&mut self) {
        let parsed = &mut self.parsed;
        if parsed.fields.len() == 1
            && let Some(message) = parsed.fields.get_mut("message")
        {
            message.logparse();
        }
    }

    fn main_text(&self, job: &mut StrBuilder) {
        let barsed = self.parsed();
        if let Some(hop) = barsed.hop_message() {
            let text = match hop {
                HopMessageKind::Enter => {
                    if self.state.hide_children {
                        "\u{2193}"
                    } else {
                        "\u{2192}"
                    }
                }
                HopMessageKind::Exit => "\u{2190}",
            };
            job.append(text, 0.0, text_format_color(SPAN));
        }

        if job.app_state.timestamps || job.app_state.matches_search(&barsed.timestamp) {
            job.append(&barsed.timestamp, 0.0, text_format_color(TIMESTAMP));
            job.append(" ", 0.0, text_format());
        }
        if job.app_state.log_levels || job.app_state.matches_search(&barsed.level) {
            let color = log_level_color(&barsed.level);
            job.append(&barsed.level, 0.0, text_format_color(color));
            job.append(" ", 0.0, text_format());
        }
        let target_displayed =
            job.app_state.targets || job.app_state.matches_search(&barsed.target);
        if target_displayed {
            job.append(&barsed.target, 0.0, text_format_color(TARGET));
        }
        if let Some(JsonValue::String(name)) = barsed.span.get("name") {
            if target_displayed {
                job.append("::", 0.0, text_format_color(COLON_COLON));
            }
            job.append(name, 0.0, text_format_color(SPAN_NAME));
        } else if !target_displayed {
            // TODO: DRY
            job.append(&barsed.target, 0.0, text_format_color(TARGET));
        }
        if barsed.fields.len() == 1
            && let Some(hop) = barsed.hop_message()
        {
            let text = match hop {
                HopMessageKind::Enter => {
                    if self.state.hide_children {
                        " hidden span:"
                    } else {
                        " enter span:"
                    }
                }
                HopMessageKind::Exit => " exit span:",
            };
            job.append(text, 0.0, text_format_color(SPAN));
            // enter/exit messages get span
            self.dict(job, INDENT * 2.0, SPAN, &barsed.span);
        } else {
            self.dict(job, INDENT, FIELD_KEYS, &barsed.fields);
        }
    }

    fn filename(&self, job: &mut StrBuilder) {
        let parsed = self.parsed();
        job.append("\n", 0.0, text_format());
        job.append(&parsed.filename, INDENT, text_format_color(FILENAME));
        job.append(":", 0.0, text_format_color(FILENAME));
        job.append(
            &format!("{}", parsed.line_number),
            0.0,
            text_format_color(FILENAME),
        );
    }

    fn spans(&self, job: &mut StrBuilder) {
        // spans are reversed from the normal stack trace
        for span in self.parsed().spans.iter().rev() {
            job.append("\n", 0.0, text_format());
            let name = match span.get("name") {
                Some(JsonValue::String(name)) => name,
                _ => "---",
            };
            job.append(name, INDENT, text_format_color(SPAN_NAME));
            self.dict(job, INDENT * 2.0, SPAN_KEYS, span)
        }
    }

    fn dict(
        &self,
        job: &mut StrBuilder,
        mut indent: f32,
        key_color: Color32,
        map: &HashMap<&str, JsonValue>,
    ) {
        // common case
        if map.len() == 1
            && let Some(message) = map.get("message")
        {
            job.append(" ", 0.0, text_format());
            message.format(job);
            return;
        }

        let total: usize = map
            .iter()
            .map(|(k, v)| {
                k.len()
                    + match v {
                        JsonValue::Bool(false) => 5,
                        JsonValue::Bool(true) => 4,
                        JsonValue::Int(v) => format!("{v}").len(),
                        JsonValue::String(s) => s.len(),
                        JsonValue::Logparsed(s) => s.backing_cart().len(),
                    }
            })
            .sum();
        let sep = if total > 100 {
            "\n"
        } else {
            indent = 0.0;
            " "
        };
        for (key, value) in map {
            job.append(sep, 0.0, text_format());
            job.append(key, indent, text_format_color(key_color));
            job.append(": ", 0.0, text_format_color(key_color));
            value.format(job);
        }
    }

    fn matches_search(&self, search: &str) -> bool {
        let parsed = self.parsed();
        parsed.timestamp.contains(search)
            || parsed.target.contains(search)
            || parsed.filename.contains(search)
            || dict_matches_search(&parsed.fields, search)
            || dict_matches_search(&parsed.span, search)
            || parsed.spans.iter().any(|m| dict_matches_search(m, search))
    }
}

fn dict_matches_search(map: &HashMap<&str, JsonValue>, search: &str) -> bool {
    map.iter().any(|(k, v)| {
        k.contains(search)
            || match v {
                JsonValue::Bool(_) => false,
                JsonValue::Int(_) => false,
                JsonValue::String(s) => s.contains(search),
                JsonValue::Logparsed(logparsed) => logparsed.backing_cart().contains(search),
            }
    })
}

#[derive(PartialEq, Eq)]
enum HopMessageKind {
    Enter,
    Exit,
}

struct StrBuilder<'a> {
    job: LayoutJob,
    app_state: &'a mut AppState,
    found_search: bool,
}

impl StrBuilder<'_> {
    fn append(&mut self, text: &str, mut leading_space: f32, format: TextFormat) {
        let mut last_ind = 0;
        if !self.app_state.search.is_empty() {
            for (ind, match_text) in text.match_indices(&self.app_state.search) {
                let prefix = &text[last_ind..ind];
                if !prefix.is_empty() {
                    let fmt = format.clone();
                    self.job.append(&text[last_ind..ind], leading_space, fmt);
                    leading_space = 0.0;
                }
                let mut fmt = format.clone();
                fmt.background = HLSEARCH;
                // section.format.background = HLSEARCH;
                // section.leading_space = 0.0;
                // suffix.leading_space = 0.0;
                self.job
                    .append(&text[ind..(ind + match_text.len())], leading_space, fmt);
                leading_space = 0.0;
                last_ind = ind + match_text.len();
                self.found_search = true;
            }
        }
        self.job.append(&text[last_ind..], leading_space, format)
    }
}

#[derive(Default, yoke::Yokeable)]
struct ParsedMessage<'b> {
    timestamp: &'b str,
    level: &'b str,
    fields: HashMap<&'b str, JsonValue<'b>>,
    target: &'b str,
    filename: &'b str,
    line_number: u64,
    span: HashMap<&'b str, JsonValue<'b>>,
    spans: Vec<HashMap<&'b str, JsonValue<'b>>>,
}

impl<'b> ParsedMessage<'b> {
    fn hop_message(&self) -> Option<HopMessageKind> {
        self.fields.get("message").and_then(|v| {
            let v: &str = match v {
                JsonValue::String(v) => v,
                JsonValue::Logparsed(v) => v.backing_cart(),
                _ => return None,
            };
            match v {
                "enter" => Some(HopMessageKind::Enter),
                "exit" => Some(HopMessageKind::Exit),
                _ => None,
            }
        })
    }
}

enum JsonValue<'b> {
    Bool(bool),
    Int(u64),
    String(&'b str),
    // wrap in box to ensure enum stays 24 bytes
    Logparsed(Box<Yoke<LogparsyYoke<'static>, &'b str>>),
}

const _: [(); 24] = [(); std::mem::size_of::<JsonValue>()];

#[derive(yoke::Yokeable)]
struct LogparsyYoke<'a>(Result<Vec<logparse::Span<'a>>, String>);

impl<'b> JsonValue<'b> {
    fn logparse(&mut self) {
        let slf = std::mem::replace(self, JsonValue::String(""));
        *self = slf.logparse_self();
    }

    fn logparse_self(self) -> Self {
        if let Self::String(msg) = self {
            let yoke = Yoke::attach_to_cart(msg, |l| {
                LogparsyYoke(
                    logparse::parse_input(l).map(|v| logparse::into_spans(v, LOGPARSE_CONFIG)),
                )
            });
            Self::Logparsed(Box::new(yoke))
        } else {
            self
        }
    }

    fn format(&self, job: &mut StrBuilder) {
        let value = match self {
            &JsonValue::Bool(v) => {
                if v {
                    "true"
                } else {
                    "false"
                }
            }
            &JsonValue::Int(v) => &format!("{v}"),
            JsonValue::String(cow) => &**cow,
            JsonValue::Logparsed(logparsed) => {
                let parsed = logparsed.get();
                match parsed {
                    LogparsyYoke(Ok(parsed)) => {
                        for span in parsed {
                            let fmt = text_format_color(logparse_color(span.kind));
                            job.append(&span.text, 0.0, fmt);
                        }
                    }
                    LogparsyYoke(Err(err)) => {
                        job.append(err, 0.0, text_format_color(LOGPARSE_ERROR));
                        job.append(" ", 0.0, text_format_color(LOGPARSE_ERROR));
                        job.append(
                            logparsed.backing_cart(),
                            0.0,
                            text_format_color(FIELD_VALUES),
                        );
                    }
                }
                return;
            }
        };
        job.append(value, 0.0, text_format_color(FIELD_VALUES));
    }
}

#[derive(Debug)]
struct ParseError {
    line: String,
    message: String,
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!(
            "parse error: {} -- on line {}",
            self.message, self.line
        ))
    }
}

impl Error for ParseError {}

fn custom_parse<'a, 'b>(bump: &'b Bump, s: &'a str) -> Result<ParsedMessage<'b>, ParseError> {
    custom_parse2(bump, &mut s.chars()).map_err(|inner| ParseError {
        line: s.to_string(),
        message: format!("{}", inner),
    })
}

enum InnerParseError<'a, 'b> {
    UnknownField(&'b str),
    UnexpectedEof(&'static str),
    UnexpectedChar(&'static [char], char),
    UnexpectedCharSingle(char, char),
    InvalidNumber(&'a str),
}

impl Display for InnerParseError<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownField(field) => f.write_fmt(format_args!("unknown field {}", field)),
            Self::UnexpectedEof(kind) => {
                f.write_fmt(format_args!("unexpected eof parsing {}", kind))
            }
            Self::UnexpectedChar(expected, got) => f.write_fmt(format_args!(
                "unexpected character '{}' (expected one of {:?})",
                got, expected
            )),
            Self::UnexpectedCharSingle(expected, got) => f.write_fmt(format_args!(
                "unexpected character '{}' (expected '{}')",
                got, expected
            )),
            Self::InvalidNumber(number) => f.write_fmt(format_args!("invalid number {}", number)),
        }
    }
}

fn custom_parse2<'a, 'b>(
    bump: &'b Bump,
    iter: &mut Chars<'a>,
) -> Result<ParsedMessage<'b>, InnerParseError<'a, 'b>> {
    let mut result = ParsedMessage::default();

    expect(iter, '{')?;

    loop {
        let key = parse_string(bump, iter)?;
        expect(iter, ':')?;
        match &*key {
            "timestamp" => result.timestamp = parse_string(bump, iter)?,
            "level" => result.level = parse_string(bump, iter)?,
            "fields" => parse_dict(bump, iter, &mut result.fields)?,
            "target" => result.target = parse_string(bump, iter)?,
            "filename" => result.filename = parse_string(bump, iter)?,
            "line_number" => result.line_number = parse_number(iter)?,
            "span" => parse_dict(bump, iter, &mut result.span)?,
            "spans" => parse_dicts(bump, iter, &mut result.spans)?,
            _ => break Err(InnerParseError::UnknownField(key)),
        }
        match iter
            .next()
            .ok_or(InnerParseError::UnexpectedEof("message"))?
        {
            '}' => break Ok(result),
            ',' => continue,
            ch => break Err(InnerParseError::UnexpectedChar(&['}', ','], ch)),
        }
    }
}

fn parse_dicts<'a, 'b>(
    bump: &'b Bump,
    iter: &mut Chars<'a>,
    result: &mut Vec<HashMap<&'b str, JsonValue<'b>>>,
) -> Result<(), InnerParseError<'a, 'b>> {
    expect(iter, '[')?;
    let mut iter_clone = iter.clone();
    if iter_clone.next() == Some(']') {
        *iter = iter_clone;
        return Ok(());
    }
    loop {
        let mut single = Default::default();
        parse_dict(bump, iter, &mut single)?;
        result.push(single);
        match iter
            .next()
            .ok_or(InnerParseError::UnexpectedEof("dict array"))?
        {
            ']' => break Ok(()),
            ',' => continue,
            ch => break Err(InnerParseError::UnexpectedChar(&[']', ','], ch)),
        }
    }
}

fn parse_dict<'a, 'b>(
    bump: &'b Bump,
    iter: &mut Chars<'a>,
    result: &mut HashMap<&'b str, JsonValue<'b>>,
) -> Result<(), InnerParseError<'a, 'b>> {
    expect(iter, '{')?;
    loop {
        let key = parse_string(bump, iter)?;
        expect(iter, ':')?;
        let start = iter.as_str();
        let value = match iter
            .next()
            .ok_or(InnerParseError::UnexpectedEof("dict key"))?
        {
            '"' => JsonValue::String(parse_string_after_quote(bump, iter)?),
            't' => {
                expect(iter, 'r')?;
                expect(iter, 'u')?;
                expect(iter, 'e')?;
                JsonValue::Bool(true)
            }
            'f' => {
                expect(iter, 'a')?;
                expect(iter, 'l')?;
                expect(iter, 's')?;
                expect(iter, 'e')?;
                JsonValue::Bool(false)
            }
            ch if ch.is_ascii_digit() => JsonValue::Int(parse_number_after_digit(start, iter)?),
            ch => break Err(InnerParseError::UnexpectedChar(&['"', 't', 'f', '0'], ch)),
        };
        result.insert(key, value);
        match iter.next().ok_or(InnerParseError::UnexpectedEof("dict"))? {
            '}' => break Ok(()),
            ',' => continue,
            ch => break Err(InnerParseError::UnexpectedChar(&['}', ','], ch)),
        }
    }
}

fn parse_string<'a, 'b>(
    bump: &'b Bump,
    iter: &mut Chars<'a>,
) -> Result<&'b str, InnerParseError<'a, 'b>> {
    expect(iter, '"')?;
    parse_string_after_quote(bump, iter)
}

fn parse_string_after_quote<'a, 'b>(
    bump: &'b Bump,
    iter: &mut Chars<'a>,
) -> Result<&'b str, InnerParseError<'a, 'b>> {
    let str_begin = iter.as_str();
    let mut backslash_count = 0;
    let mut cur_index = 0;
    let end_index = loop {
        let Some(index) = str_begin[cur_index..].find(['"', '\\']) else {
            return Err(InnerParseError::UnexpectedEof("string"));
        };
        let index = index + cur_index;
        if str_begin.as_bytes()[index] == b'"' {
            if backslash_count == 0 {
                // There are no backslashes. Use a direct bumpalo alloc_str to copy the data into
                // bumpalo.
                *iter = str_begin[(index + 1)..].chars();
                return Ok(bump.alloc_str(&str_begin[..index]));
            } else {
                // There are backslashes. Do a custom alloc_slice_fill_with to skip over the
                // backslashes while copying.
                break index;
            }
        } else {
            backslash_count += 1;
            let mut asdf = str_begin[(index + 1)..].chars();
            // skip the escaped char
            if let None = asdf.next() {
                return Err(InnerParseError::UnexpectedEof("string backslash"));
            }
            cur_index = str_begin.len() - asdf.as_str().len();
        }
    };

    *iter = str_begin[(end_index + 1)..].chars();
    let mut i = 0;
    let fullslice = str_begin[..end_index].as_bytes();
    let result = bump.alloc_slice_fill_with(end_index - backslash_count, |_| {
        let mut result = fullslice[i];
        // backslash is a single ascii character, so it can be detected as a u8
        if result == b'\\' {
            i += 1;
            result = fullslice[i];
            // backslash followed by a backslash is a single ascii character to skip over, so we
            // won't detect it on the next iteration (we don't process any more complex escapes)
        }
        i += 1;
        result
    });
    debug_assert!(i == fullslice.len());
    // from_utf8_unchecked does not seem to improve perf much, even though it would be
    // theoretically safe to do so
    Ok(str::from_utf8(result).unwrap())
}

fn parse_number<'a, T: FromStr>(iter: &mut Chars<'a>) -> Result<T, InnerParseError<'a, 'static>>
where
    <T as FromStr>::Err: Debug,
{
    let start = iter.as_str();
    let ch = iter
        .next()
        .ok_or(InnerParseError::UnexpectedEof("number"))?;
    if ch.is_ascii_digit() {
        parse_number_after_digit(start, iter)
    } else {
        Err(InnerParseError::UnexpectedChar(&['0'], ch))
    }
}

fn parse_number_after_digit<'a, T: FromStr>(
    start: &'a str,
    iter: &mut Chars<'a>,
) -> Result<T, InnerParseError<'a, 'static>>
where
    <T as FromStr>::Err: Debug,
{
    let slice = loop {
        let mut cloned = iter.clone();
        let Some(ch) = cloned.next() else {
            break start;
        };
        if !ch.is_ascii_digit() {
            let len = start.len() - iter.as_str().len();
            break &start[..len];
        }
        *iter = cloned;
    };
    T::from_str(slice).map_err(|_| InnerParseError::InvalidNumber(slice))
}

fn expect<'a>(iter: &mut Chars<'a>, ch: char) -> Result<(), InnerParseError<'a, 'static>> {
    if let Some(got) = iter.next() {
        if got == ch {
            Ok(())
        } else {
            Err(InnerParseError::UnexpectedCharSingle(ch, got))
        }
    } else {
        Err(InnerParseError::UnexpectedCharSingle(ch, '\0'))
    }
}
