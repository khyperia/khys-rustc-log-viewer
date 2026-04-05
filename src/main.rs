use bumpalo::Bump;
use eframe::{
    CreationContext,
    egui::{
        CentralPanel, Color32, ComboBox, FontId, Frame, InputState, Key, Modifiers, Panel,
        TextFormat, Ui, UiBuilder, text::LayoutJob,
    },
};
use std::{
    error::Error,
    fmt::{self, Debug, Display, Formatter},
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
    str::Chars,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SendError, TryRecvError},
    },
    time::Instant,
};

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
const SPAN_KEYS: Color32 = alacritty::YELLOW;
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

    fn vdict_matches_search(&self, list: &LinkedList<Span>) -> bool {
        if self.search.is_empty() {
            false
        } else {
            list.iter()
                .any(|map| span_matches_search(map, &self.search))
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
                let end = self.end_time.unwrap_or_else(Instant::now);
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
            self.exclude ^ self.matches_raw(message)
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
    LineNumber,
    SpanName,
    Spans,
}

impl FilterKind {
    fn run(&self, message: &Message, mut visit: impl FnMut(&str) -> bool) -> bool {
        match self {
            Self::Timestamp => visit(message.parsed.timestamp),
            Self::Level => visit(message.parsed.level),
            Self::Fields => message
                .parsed
                .fields
                .iter()
                .any(|(k, v)| visit(k) || visit(v)),
            Self::Target => visit(message.parsed.target),
            Self::Filename => visit(message.parsed.filename),
            Self::LineNumber => visit(message.parsed.line_number),
            Self::SpanName => message.parsed.span.name.is_some_and(visit),
            Self::Spans => message.parsed.spans.iter().any(|n| {
                n.name.is_some_and(&mut visit) || n.map.iter().any(|(k, v)| visit(k) || visit(v))
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
            Self::LineNumber => "line number",
            Self::SpanName => "span name",
            Self::Spans => "spans",
        }
    }

    const ALL: [Self; 8] = [
        Self::Timestamp,
        Self::Level,
        Self::Fields,
        Self::Target,
        Self::Filename,
        Self::LineNumber,
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
    // Box, to keep the size of the struct down
    logparsed_message: Option<Box<Result<Vec<logparse::Span<'b>>, String>>>,
    parent: Option<usize>,
    indent: usize,
    state: MessageState,
}

const _: [(); std::mem::size_of::<usize>() * 20] = [(); std::mem::size_of::<Message>()];

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
            logparsed_message: None,
            parent,
            indent: self_indent,
            state: Default::default(),
        })
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
        self.build_text(&mut job);
        let rsp = if !job.found_search
            && !app_state.search.is_empty()
            && self.matches_search(&app_state.search)
        {
            job.found_search = true;
            // fallback to highlight the whole message if we're not displaying the matching text
            Frame::NONE
                .fill(HLSEARCH)
                .show(ui, |ui| ui.label(job.job.clone()))
                .inner
        } else {
            ui.label(job.job.clone())
        };
        if job.found_search && ui.clip_rect().intersects(rsp.rect) {
            app_state.search_onscreen = true;
        }
        rsp.context_menu(|ui| {
            if let Some(HopMessageKind::Enter) = self.parsed.hop_message() {
                ui.checkbox(&mut self.state.hide_children, "hide children");
            } else if let Some(parent) = parent {
                ui.checkbox(&mut parent.state.hide_children, "hide siblings");
            }
            ui.checkbox(&mut self.state.display_filename, "filename");
            if let LinkedList::Empty = self.parsed.spans {
            } else {
                ui.checkbox(&mut self.state.display_spans, "spans");
            }
            if ui.button("exclude target filter").clicked() {
                app_state.filters.push(Filter {
                    kind: FilterKind::Target,
                    filter: self.parsed.target.to_string(),
                    exclude: true,
                });
            }
        });
    }

    fn logparse_single_message(&mut self) {
        if self.logparsed_message.is_none()
            && let JsonMap::Cons {
                next: JsonMap::Empty,
                item: ("message", message),
            } = &self.parsed.fields
        {
            self.logparsed_message = Some(Box::new(
                logparse::parse_input(message).map(|v| logparse::into_spans(v, LOGPARSE_CONFIG)),
            ));
        }
    }

    fn build_text(&self, job: &mut StrBuilder) {
        if let Some(hop) = self.parsed.hop_message() {
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

        if job.app_state.timestamps || job.app_state.matches_search(self.parsed.timestamp) {
            job.append(self.parsed.timestamp, 0.0, text_format_color(TIMESTAMP));
            job.append(" ", 0.0, text_format());
        }
        if job.app_state.log_levels || job.app_state.matches_search(self.parsed.level) {
            let color = log_level_color(self.parsed.level);
            job.append(self.parsed.level, 0.0, text_format_color(color));
            job.append(" ", 0.0, text_format());
        }
        let target_displayed =
            job.app_state.targets || job.app_state.matches_search(self.parsed.target);
        if target_displayed {
            job.append(self.parsed.target, 0.0, text_format_color(TARGET));
        }
        if let Some(name) = self.parsed.span.name {
            if target_displayed {
                job.append("::", 0.0, text_format_color(COLON_COLON));
            }
            job.append(name, 0.0, text_format_color(SPAN_NAME));
        } else if !target_displayed {
            // TODO: DRY
            job.append(self.parsed.target, 0.0, text_format_color(TARGET));
        }
        self.fields(job);
        if self.state.display_filename
            || !job.found_search
                && (job.app_state.matches_search(self.parsed.filename)
                    || job.app_state.matches_search(self.parsed.line_number))
        {
            self.filename(job);
        }
        if self.state.display_spans
            || !job.found_search && job.app_state.vdict_matches_search(self.parsed.spans)
        {
            self.spans(job);
        }
    }

    fn fields(&self, job: &mut StrBuilder) {
        if let JsonMap::Cons {
            next: JsonMap::Empty,
            ..
        } = &self.parsed.fields
            && let Some(hop) = self.parsed.hop_message()
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
            self.dict(job, INDENT * 2.0, SPAN_KEYS, self.parsed.span.map);
        } else {
            if let Some(logparsed) = self.logparsed_message.as_deref() {
                job.append(" ", 0.0, text_format());
                match logparsed {
                    Ok(parsed) => {
                        for span in parsed {
                            let fmt = text_format_color(logparse_color(span.kind));
                            job.append(&span.text, 0.0, fmt);
                        }
                        return;
                    }
                    Err(err) => job.append(err, 0.0, text_format_color(LOGPARSE_ERROR)),
                }
            }

            // common case
            if let JsonMap::Cons {
                next: JsonMap::Empty,
                item: ("message", value),
            } = &self.parsed.fields
            {
                job.append(" ", 0.0, text_format());
                job.append(value, 0.0, text_format_color(FIELD_VALUES));
                return;
            }

            self.dict(job, INDENT, FIELD_KEYS, self.parsed.fields);
        }
    }

    fn filename(&self, job: &mut StrBuilder) {
        job.append("\n", 0.0, text_format());
        job.append(self.parsed.filename, INDENT, text_format_color(FILENAME));
        job.append(":", 0.0, text_format_color(FILENAME));
        job.append(self.parsed.line_number, 0.0, text_format_color(FILENAME));
    }

    fn spans(&self, job: &mut StrBuilder) {
        for span in self.parsed.spans.iter() {
            job.append("\n", 0.0, text_format());
            let name = span.name.unwrap_or("---");
            job.append(name, INDENT, text_format_color(SPAN_NAME));
            self.dict(job, INDENT * 2.0, SPAN_KEYS, span.map)
        }
    }

    fn dict(&self, job: &mut StrBuilder, mut indent: f32, key_color: Color32, map: &JsonMap) {
        let total: usize = map.iter().map(|(k, v)| k.len() + v.len()).sum();
        let sep = if total > 100 {
            "\n"
        } else {
            indent = 0.0;
            " "
        };
        for (key, value) in map.iter() {
            job.append(sep, 0.0, text_format());
            job.append(key, indent, text_format_color(key_color));
            job.append(": ", 0.0, text_format_color(key_color));
            job.append(value, 0.0, text_format_color(FIELD_VALUES));
        }
    }

    fn matches_search(&self, search: &str) -> bool {
        self.parsed.timestamp.contains(search)
            || self.parsed.target.contains(search)
            || self.parsed.filename.contains(search)
            || self.parsed.line_number.contains(search)
            || dict_matches_search(self.parsed.fields, search)
            || span_matches_search(&self.parsed.span, search)
            || self
                .parsed
                .spans
                .iter()
                .any(|m| span_matches_search(m, search))
    }
}

fn span_matches_search(map: &Span, search: &str) -> bool {
    map.name.is_some_and(|n| n.contains(search)) || dict_matches_search(map.map, search)
}

fn dict_matches_search(map: &JsonMap, search: &str) -> bool {
    map.iter()
        .any(|(k, v)| k.contains(search) || v.contains(search))
}

#[derive(PartialEq, Eq)]
enum HopMessageKind {
    Enter,
    Exit,
}

struct StrBuilder<'a> {
    job: LayoutJob,
    app_state: &'a AppState,
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

#[derive(Default)]
struct ParsedMessage<'b> {
    timestamp: &'b str,
    level: &'b str,
    fields: &'b JsonMap<'b>,
    target: &'b str,
    filename: &'b str,
    line_number: &'b str,
    span: Span<'b>,
    spans: &'b LinkedList<'b, Span<'b>>,
}

impl<'b> ParsedMessage<'b> {
    fn hop_message(&self) -> Option<HopMessageKind> {
        if let JsonMap::Cons {
            next: JsonMap::Empty,
            item: ("message", value),
        } = *self.fields
        {
            match value {
                "enter" => Some(HopMessageKind::Enter),
                "exit" => Some(HopMessageKind::Exit),
                _ => None,
            }
        } else {
            None
        }
    }
}

#[derive(Default)]
struct Span<'b> {
    name: Option<&'b str>,
    map: &'b JsonMap<'b>,
}

enum LinkedList<'b, T> {
    Empty,
    Cons {
        next: &'b LinkedList<'b, T>,
        item: T,
    },
}

type JsonMap<'b> = LinkedList<'b, (&'b str, &'b str)>;

impl<'b, T> LinkedList<'b, T> {
    fn iter(&'b self) -> LinkedListIter<'b, T> {
        LinkedListIter { entry: self }
    }
}

impl<'b, T> Default for &'b LinkedList<'b, T> {
    fn default() -> Self {
        &LinkedList::Empty
    }
}

struct LinkedListIter<'b, T> {
    entry: &'b LinkedList<'b, T>,
}

impl<'b, T> Iterator for LinkedListIter<'b, T> {
    type Item = &'b T;
    fn next(&mut self) -> Option<Self::Item> {
        match self.entry {
            LinkedList::Empty => None,
            LinkedList::Cons { next, item } => {
                self.entry = *next;
                Some(item)
            }
        }
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

fn custom_parse<'b>(bump: &'b Bump, s: &str) -> Result<ParsedMessage<'b>, ParseError> {
    custom_parse2(bump, &mut s.chars()).map_err(|inner| ParseError {
        line: s.to_string(),
        message: format!("{}", inner),
    })
}

enum InnerParseError<'b> {
    UnknownField(&'b str),
    UnexpectedEof(&'static str),
    UnexpectedChar(&'static [char], char),
    UnexpectedCharSingle(char, char),
}

impl Display for InnerParseError<'_> {
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
        }
    }
}

fn custom_parse2<'a, 'b>(
    bump: &'b Bump,
    iter: &mut Chars<'a>,
) -> Result<ParsedMessage<'b>, InnerParseError<'b>> {
    let mut result = ParsedMessage::default();

    expect(iter, '{')?;

    loop {
        let key = parse_string(bump, iter)?;
        expect(iter, ':')?;
        match key {
            "timestamp" => result.timestamp = parse_string(bump, iter)?,
            "level" => result.level = parse_string(bump, iter)?,
            "fields" => parse_dict(bump, iter, &mut result.fields, |_, _| false)?,
            "target" => result.target = parse_string(bump, iter)?,
            "filename" => result.filename = parse_string(bump, iter)?,
            "line_number" => result.line_number = parse_number(bump, iter)?,
            "span" => parse_span(bump, iter, &mut result.span)?,
            "spans" => parse_spans(bump, iter, &mut result.spans)?,
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

fn parse_spans<'a, 'b>(
    bump: &'b Bump,
    iter: &mut Chars<'a>,
    result: &mut &'b LinkedList<'b, Span<'b>>,
) -> Result<(), InnerParseError<'b>> {
    expect(iter, '[')?;
    let mut iter_clone = iter.clone();
    if iter_clone.next() == Some(']') {
        *iter = iter_clone;
        return Ok(());
    }
    loop {
        let mut single = Default::default();
        parse_span(bump, iter, &mut single)?;
        let cons = LinkedList::Cons {
            next: result,
            item: single,
        };
        *result = bump.alloc(cons);
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

fn parse_span<'a, 'b>(
    bump: &'b Bump,
    iter: &mut Chars<'a>,
    result: &mut Span<'b>,
) -> Result<(), InnerParseError<'b>> {
    parse_dict(bump, iter, &mut result.map, |key, value| {
        if key == "name" {
            result.name = Some(value);
            true
        } else {
            false
        }
    })
}

fn parse_dict<'a, 'b>(
    bump: &'b Bump,
    iter: &mut Chars<'a>,
    result: &mut &'b JsonMap<'b>,
    mut on_key_value: impl FnMut(&'b str, &'b str) -> bool,
) -> Result<(), InnerParseError<'b>> {
    expect(iter, '{')?;
    loop {
        let key = parse_string(bump, iter)?;
        expect(iter, ':')?;
        let start = iter.as_str();
        let value = match iter
            .next()
            .ok_or(InnerParseError::UnexpectedEof("dict key"))?
        {
            '"' => parse_string_after_quote(bump, iter)?,
            't' => {
                expect(iter, 'r')?;
                expect(iter, 'u')?;
                expect(iter, 'e')?;
                "true"
            }
            'f' => {
                expect(iter, 'a')?;
                expect(iter, 'l')?;
                expect(iter, 's')?;
                expect(iter, 'e')?;
                "false"
            }
            ch if ch.is_ascii_digit() => parse_number_after_digit(bump, start, iter)?,
            ch => break Err(InnerParseError::UnexpectedChar(&['"', 't', 'f', '0'], ch)),
        };
        if !on_key_value(key, value) {
            let cons = JsonMap::Cons {
                next: result,
                item: (key, value),
            };
            *result = bump.alloc(cons);
        }
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
) -> Result<&'b str, InnerParseError<'b>> {
    expect(iter, '"')?;
    parse_string_after_quote(bump, iter)
}

fn parse_string_after_quote<'a, 'b>(
    bump: &'b Bump,
    iter: &mut Chars<'a>,
) -> Result<&'b str, InnerParseError<'b>> {
    let str_begin = iter.as_str();
    let mut backslash_count = 0;
    let mut index = 0;
    let end_index = loop {
        if let Some(tmp) = str_begin[index..].find(['"', '\\']) {
            index += tmp;
        } else {
            return Err(InnerParseError::UnexpectedEof("string"));
        }
        if str_begin.as_bytes()[index] == b'\\' {
            backslash_count += 1;
            let mut char_skipper = str_begin[(index + 1)..].chars();
            // skip the escaped char
            if char_skipper.next().is_none() {
                return Err(InnerParseError::UnexpectedEof("string backslash"));
            }
            index = str_begin.len() - char_skipper.as_str().len();
        } else {
            *iter = str_begin[(index + 1)..].chars();
            if backslash_count == 0 {
                // There are no backslashes. Use a direct bumpalo alloc_str to copy the data into
                // bumpalo.
                return Ok(bump.alloc_str(&str_begin[..index]));
            } else {
                // There are backslashes. Do a custom alloc_slice_fill_with to skip over the
                // backslashes while copying.
                break index;
            }
        }
    };

    let mut i = 0;
    let fullslice = &str_begin.as_bytes()[..end_index];
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

fn parse_number<'a, 'b>(
    bump: &'b Bump,
    iter: &mut Chars<'a>,
) -> Result<&'b str, InnerParseError<'static>> {
    let start = iter.as_str();
    let ch = iter
        .next()
        .ok_or(InnerParseError::UnexpectedEof("number"))?;
    if ch.is_ascii_digit() {
        parse_number_after_digit(bump, start, iter)
    } else {
        Err(InnerParseError::UnexpectedChar(&['0'], ch))
    }
}

fn parse_number_after_digit<'a, 'b>(
    bump: &'b Bump,
    start: &'a str,
    iter: &mut Chars<'a>,
) -> Result<&'b str, InnerParseError<'static>> {
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
    Ok(bump.alloc_str(slice))
}

fn expect<'a>(iter: &mut Chars<'a>, ch: char) -> Result<(), InnerParseError<'static>> {
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
