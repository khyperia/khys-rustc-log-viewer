use bumpalo::Bump;
use eframe::{
    CreationContext, NativeOptions,
    egui::{
        CentralPanel, Color32, FontId, Frame, Key, Modifiers, Panel, TextFormat, Ui, UiBuilder,
        text::{LayoutJob, LayoutSection},
    },
    egui_wgpu,
};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    net::{SocketAddr, TcpListener, ToSocketAddrs},
    num::NonZeroUsize,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SendError, Sender, TryRecvError},
    },
    time::Instant,
};

use crate::filter::{Filter, FilterKind};

mod filter;
mod parser;

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
        "WARN" => alacritty::BRIGHT_YELLOW,
        "ERROR" => alacritty::BRIGHT_RED,
        _ => alacritty::RED,
    }
}

fn logparse_color(kind: logparse::SpanKind) -> Color32 {
    match kind {
        logparse::SpanKind::Number | logparse::SpanKind::Literal | logparse::SpanKind::Lifetime => {
            alacritty::CYAN
        }
        logparse::SpanKind::String => alacritty::YELLOW,
        logparse::SpanKind::Constructor => alacritty::BLUE,
        logparse::SpanKind::Surroundings => alacritty::DIM_CYAN,
        logparse::SpanKind::Separator | logparse::SpanKind::Path => alacritty::DIM_WHITE,
        logparse::SpanKind::Delimiter(_)
        | logparse::SpanKind::Space(_)
        | logparse::SpanKind::Text => alacritty::WHITE,
    }
}

const INDENT: f32 = 8.0;

const LOGPARSE_CONFIG: logparse::Config = logparse::Config {
    collapse_space: false,
    pretty_print: None,
};

fn main() -> anyhow::Result<()> {
    let Some(data_source) = parse_cmdline() else {
        return Ok(());
    };

    let mut opts = NativeOptions::default();
    match &mut opts.wgpu_options.wgpu_setup {
        egui_wgpu::WgpuSetup::CreateNew(setup) => {
            let old = std::mem::replace(&mut setup.device_descriptor, Arc::new(|_| unreachable!()));
            setup.device_descriptor = Arc::new(move |adapter| {
                let mut descr = old(adapter);
                // Oli has no dedicated GPU
                descr.required_limits.max_texture_dimension_3d = 512;
                descr
            })
        }
        eframe::egui_wgpu::WgpuSetup::Existing(_) => unreachable!(),
    }

    let mut bump = Bump::new();
    std::thread::scope(|scope| {
        eframe::run_native(
            "Khy's rustc log viewer",
            opts,
            Box::new(|cc| Ok(Box::new(App::new(cc, &mut bump, scope, data_source)))),
        )
    })?;
    Ok(())
}

enum DataSource {
    File(PathBuf),
    Socket(SocketDataSource),
}

struct SocketDataSource {
    socket_addr: std::vec::IntoIter<SocketAddr>,
    fork_after_listen: Vec<String>,
}

fn parse_cmdline() -> Option<DataSource> {
    let mut args = std::env::args().skip(1);
    if let Some(first) = args.next() {
        if let Some(second) = args.next() {
            if first == "--tcp" {
                match second.to_socket_addrs() {
                    Ok(socket_addr) => {
                        let fork_after_listen = args.collect();
                        let socket = SocketDataSource {
                            socket_addr,
                            fork_after_listen,
                        };
                        Some(DataSource::Socket(socket))
                    }
                    Err(e) => {
                        println!("couldn't parse tcp address: {e}");
                        None
                    }
                }
            } else {
                println!(
                    "Only one argument is allowed (the path that was given to RUSTC_LOG_OUTPUT_TARGET)"
                );
                None
            }
        } else {
            Some(DataSource::File(PathBuf::from(first)))
        }
    } else {
        if let Ok(targ) = std::env::var("RUSTC_LOG_OUTPUT_TARGET") {
            Some(DataSource::File(PathBuf::from(targ)))
        } else {
            println!("No file provided. Pass the file that was given to RUSTC_LOG_OUTPUT_TARGET");
            None
        }
    }
}

struct App<'b> {
    messages: Vec<Message<'b>>,
    messages_reader: Receiver<Message<'b>>,
    ui_has_been_notified: Arc<AtomicBool>,
    start_time: Option<Instant>,
    end_time: Option<Instant>,
    state: AppState,
}

#[derive(Default)]
struct AppState {
    scroll_value: scroller::ScrollValue,
    entering_search_text: bool,
    search: String,
    search_onscreen: bool,
    timestamps: bool,
    log_levels: bool,
    targets: bool,
    filters: Vec<Filter>,
    filter_save_result: String,
}

impl AppState {
    fn new() -> Self {
        let (filters, filter_save_result) = match Filter::from_file() {
            Ok(filters) => (filters, String::new()),
            Err(e) => (vec![], e),
        };
        Self {
            log_levels: true,
            targets: true,
            filters,
            filter_save_result,
            ..Self::default()
        }
    }

    fn matches_search(&self, s: &str) -> bool {
        if self.search.is_empty() {
            false
        } else {
            s.contains(&self.search)
        }
    }
}

impl<'b> App<'b> {
    fn new<'scope, 'env>(
        cc: &CreationContext,
        bump: &'b mut Bump,
        scope: &'scope std::thread::Scope<'scope, 'env>,
        data_source: DataSource,
    ) -> Self
    where
        'b: 'scope,
    {
        let ui_has_been_notified = Arc::new(AtomicBool::new(false));
        let atomic_bool = ui_has_been_notified.clone();
        let egui_ctx = cc.egui_ctx.clone();
        let messages_reader = read_lines(bump, scope, data_source, move || {
            if !atomic_bool.swap(true, Ordering::Relaxed) {
                egui_ctx.request_repaint();
            }
        });
        App {
            messages: vec![],
            messages_reader,
            ui_has_been_notified,
            start_time: None,
            end_time: None,
            state: AppState::new(),
        }
    }

    fn recv_message(&mut self, message: Message<'b>) {
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }
        if message.parsed.hop_message() == Some(HopMessageKind::Exit)
            && let Some(parent) = message.parent
        {
            self.messages[parent].exit = NonZeroUsize::new(self.messages.len());
        }
        self.messages.push(message);
    }

    // silly nit: index is *inclusive* here
    fn next_search(&self, index: usize) -> Option<usize> {
        (index..self.messages.len()).find(|&index| self.next_search_raw(index))
    }

    // silly nit: index is *exclusive* here
    fn prev_search(&self, index: usize) -> Option<usize> {
        (0..index).rev().find(|&index| self.next_search_raw(index))
    }

    fn next_search_raw(&self, index: usize) -> bool {
        self.messages[index].is_displayed(&self.messages, &self.state)
            && (self.messages[index].matches_search(&self.state.search) || {
                let mut job = String::new();
                self.messages[index].build_text(&self.state, &mut job);
                job.contains(&self.state.search)
            })
    }
}

impl eframe::App for App<'_> {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.ui_has_been_notified.store(false, Ordering::Relaxed);
        loop {
            match self.messages_reader.try_recv() {
                Ok(message) => self.recv_message(message),
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
            scroller::scroller_mouse_input(input, &mut self.state.scroll_value);
            if self.state.entering_search_text {
                if input.consume_key(Modifiers::NONE, Key::Escape) {
                    self.state.entering_search_text = false;
                }
            } else {
                scroller::scroller_key_input(
                    input,
                    &mut self.state.scroll_value,
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
                        && let Some(index) = self.prev_search(self.state.scroll_value.index)
                    {
                        self.state.scroll_value.index = index;
                        self.state.scroll_value.pixel_offset = 0.0;
                    }
                    if input.consume_key(Modifiers::NONE, Key::N)
                        && let Some(index) = self
                            .next_search((self.state.scroll_value.index + 1) % self.messages.len())
                    {
                        self.state.scroll_value.index = index;
                        self.state.scroll_value.pixel_offset = 0.0;
                    }
                }
            }
        });
        scroller::clamp_scroller(&mut self.state.scroll_value, self.messages.len());
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
                if let Some(start_time) = self.start_time {
                    let end = self.end_time.unwrap_or_else(Instant::now);
                    ui.label(format!(
                        "{} messages in {:?}",
                        self.messages.len(),
                        end - start_time,
                    ));
                }
                ui.checkbox(&mut self.state.timestamps, "timestamps");
                ui.checkbox(&mut self.state.log_levels, "log levels");
                ui.checkbox(&mut self.state.targets, "targets");
            });
            self.state.filters.retain_mut(|q| {
                let salt = id_salt;
                id_salt += 1;
                q.ui(salt, ui)
            });
            ui.horizontal(|ui| {
                if ui.button("add filter").clicked() {
                    self.state.filters.push(Filter::new());
                }
                if ui.button("save filters as default").clicked() {
                    self.state.filter_save_result = match Filter::to_file(&self.state.filters) {
                        Ok(()) => String::new(),
                        Err(e) => e,
                    };
                }
                if !self.state.filter_save_result.is_empty() {
                    ui.label(&self.state.filter_save_result);
                }
            });
            ui.add_space(2.0);
        });
        self.state.search_onscreen = false;
        let panel = CentralPanel::default().frame(Frame::new().fill(Color32::from_rgb(38, 50, 56)));
        panel.show_inside(ui, |ui| {
            let old_scroll = self.state.scroll_value.clone();
            let mut big_adjusted_scroll = old_scroll.clone();
            scroller::big_scroller(
                ui,
                &mut big_adjusted_scroll,
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
            // only take the big scroller's edit (which is a line/pixel shuffle to keep the same
            // scroll) if the ui logic did not adjust the scroll (which is jumping around)
            if self.state.scroll_value == old_scroll
                && self.state.scroll_value != big_adjusted_scroll
            {
                self.state.scroll_value = big_adjusted_scroll;
            }
        });
        if !self.state.search.is_empty()
            && !self.state.search_onscreen
            && ensure_search_onscreen
            && let Some(index) = self.next_search(self.state.scroll_value.index)
        {
            self.state.scroll_value.index = index;
            self.state.scroll_value.pixel_offset = 0.0;
        }
    }
}

mod scroller {
    use eframe::egui::{InputState, Key, Modifiers, Ui, UiBuilder};

    #[derive(Default, Clone, PartialEq)]
    pub struct ScrollValue {
        pub index: usize,
        pub pixel_offset: f32,
    }

    pub fn clamp_scroller(value: &mut ScrollValue, len: usize) {
        if value.index == 0 && value.pixel_offset < 0.0 {
            value.pixel_offset = 0.0;
        }
        if value.index >= len && value.pixel_offset > 0.0 {
            value.pixel_offset = 0.0;
        }
    }

    pub fn scroller_mouse_input(input: &mut InputState, value: &mut ScrollValue) {
        let scroll_mult = -4.0; // inverted?
        value.pixel_offset += input.smooth_scroll_delta.y * scroll_mult;
        input.smooth_scroll_delta.y = 0.0;
    }

    pub fn scroller_key_input(
        input: &mut InputState,
        value: &mut ScrollValue,
        len: usize,
        ui_height: f32,
    ) {
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
        if input.consume_key(Modifiers::NONE, Key::End)
            || input.consume_key(Modifiers::SHIFT, Key::G)
        {
            value.index = len - 1;
        }
        if input.consume_key(Modifiers::NONE, Key::Home)
            || input.consume_key(Modifiers::NONE, Key::G)
        {
            value.index = 0;
        }
    }

    pub fn big_scroller(
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
                if end < absolute_begin && value.index < len {
                    value.index += 1;
                    let size = end - begin;
                    value.pixel_offset -= size;
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
}

fn read_lines<'b: 'scope, 'scope, 'env>(
    bump: &'b mut Bump,
    scope: &'scope std::thread::Scope<'scope, 'env>,
    data_source: DataSource,
    notify_ui_thread: impl Fn() + Send + 'static,
) -> Receiver<Message<'b>> {
    let (send, recv) = mpsc::channel();

    scope.spawn(move || match data_source {
        DataSource::File(path) => {
            let file = File::open(path).unwrap();
            parse_from_reader(bump, notify_ui_thread, send, file)
        }
        DataSource::Socket(addr) => {
            let listener = TcpListener::bind(addr.socket_addr.as_slice()).unwrap();
            let child = addr.fork_after_listen.first().map(|cmd| {
                std::process::Command::new(cmd)
                    .args(&addr.fork_after_listen[1..])
                    .spawn()
                    .expect("couldn't spawn fork-after-tcp process")
            });
            let (stream, _) = listener.accept().unwrap();
            drop(listener);
            parse_from_reader(bump, notify_ui_thread, send, stream);
            if let Some(mut child) = child {
                child.kill().expect("couldn't kill fork-after-tcp process")
            }
        }
    });

    recv
}

fn parse_from_reader<'b>(
    bump: &'b mut Bump,
    notify_ui_thread: impl Fn() + Send + 'static,
    send: Sender<Message<'b>>,
    reader: impl Read,
) {
    let mut reader = BufReader::new(reader);
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
}

#[inline]
fn text_format() -> TextFormat {
    TextFormat {
        font_id: FontId::monospace(14.0),
        ..TextFormat::default()
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
    #[allow(clippy::box_collection)]
    logparsed_message: Option<Box<Vec<logparse::Span<'b>>>>,
    parent: Option<usize>,
    // if this is an Enter message, this is the index of the corresponding Exit
    // NonZeroUsize to get a niche opti to keep the size down
    exit: Option<NonZeroUsize>,
    // can probably be a u8, but it doesn't currently help struct size to make it u8
    indent: u32,
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
        let parsed = parser::custom_parse(bump, line)?;
        let msg = parsed.hop_message();
        let parent = parent_stack.last().copied();
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
            exit: None,
            indent: self_indent.try_into().unwrap_or(u32::MAX),
            state: MessageState::default(),
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
        let mut job = LayoutJob::default();
        self.build_text(app_state, &mut job);
        let mut found_search = Self::handle_search(app_state, &mut job);
        let rsp = if !found_search
            && !app_state.search.is_empty()
            && self.matches_search(&app_state.search)
        {
            found_search = true;
            // fallback to highlight the whole message if we're not displaying the matching text
            Frame::NONE
                .fill(HLSEARCH)
                .show(ui, |ui| ui.label(job.clone()))
                .inner
        } else {
            ui.label(job.clone())
        };
        if found_search && ui.clip_rect().intersects(rsp.rect) {
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
            if ui.button("exclude target filename").clicked() {
                app_state.filters.push(Filter {
                    kind: FilterKind::Filename,
                    filter: self.parsed.filename.to_string(),
                    exclude: true,
                });
            }
            if let Some(exit) = self.exit
                && ui.button("jump to exit").clicked()
            {
                app_state.scroll_value = scroller::ScrollValue {
                    index: exit.get(),
                    pixel_offset: 0.0,
                }
            }
            if let Some(parent) = self.parent
                && ui
                    .button(if self.parsed.hop_message() == Some(HopMessageKind::Exit) {
                        "jump to enter"
                    } else {
                        "jump to parent"
                    })
                    .clicked()
            {
                app_state.scroll_value = scroller::ScrollValue {
                    index: parent,
                    pixel_offset: 0.0,
                }
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
            self.logparsed_message = Some(Box::new(match logparse::parse_input(message) {
                Ok(v) => logparse::into_spans(v, LOGPARSE_CONFIG),
                Err(e) => vec![
                    logparse::Span {
                        text: "logparse error(".into(),
                        kind: logparse::SpanKind::Text,
                    },
                    logparse::Span {
                        text: e.into(),
                        kind: logparse::SpanKind::Text,
                    },
                    logparse::Span {
                        text: "): ".into(),
                        kind: logparse::SpanKind::Text,
                    },
                    logparse::Span {
                        text: (*message).into(),
                        kind: logparse::SpanKind::Text,
                    },
                ],
            }));
        }
    }

    fn build_text(&self, app_state: &AppState, job: &mut impl Appendable) {
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

        if app_state.timestamps || app_state.matches_search(self.parsed.timestamp) {
            job.append(self.parsed.timestamp, 0.0, text_format_color(TIMESTAMP));
            job.append(" ", 0.0, text_format());
        }
        if app_state.log_levels || app_state.matches_search(self.parsed.level) {
            let color = log_level_color(self.parsed.level);
            job.append(self.parsed.level, 0.0, text_format_color(color));
            job.append(" ", 0.0, text_format());
        }
        let target_displayed = app_state.targets || app_state.matches_search(self.parsed.target);
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
        if self.state.display_filename {
            self.filename(job);
        }
        if self.state.display_spans {
            self.spans(job);
        }
    }

    fn fields(&self, job: &mut impl Appendable) {
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
            Self::dict(job, INDENT * 2.0, SPAN_KEYS, self.parsed.span.map);
        } else {
            if let Some(logparsed) = self.logparsed_message.as_deref() {
                job.append(" ", 0.0, text_format());
                for span in logparsed {
                    let fmt = text_format_color(logparse_color(span.kind));
                    job.append(&span.text, 0.0, fmt);
                }
                return;
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

            Self::dict(job, INDENT, FIELD_KEYS, self.parsed.fields);
        }
    }

    fn filename(&self, job: &mut impl Appendable) {
        job.append("\n", 0.0, text_format());
        job.append(self.parsed.filename, INDENT, text_format_color(FILENAME));
        job.append(":", 0.0, text_format_color(FILENAME));
        job.append(self.parsed.line_number, 0.0, text_format_color(FILENAME));
    }

    fn spans(&self, job: &mut impl Appendable) {
        for span in self.parsed.spans.iter() {
            job.append("\n", 0.0, text_format());
            let name = span.name.unwrap_or("---");
            job.append(name, INDENT, text_format_color(SPAN_NAME));
            Self::dict(job, INDENT * 2.0, SPAN_KEYS, span.map);
        }
    }

    fn dict(job: &mut impl Appendable, mut indent: f32, key_color: Color32, map: &JsonMap) {
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
            || self.parsed.level.contains(search)
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

    fn handle_search(app_state: &AppState, job: &mut LayoutJob) -> bool {
        if app_state.search.is_empty() {
            return false;
        }
        let mut any = false;
        for (ind, match_text) in job.text.match_indices(&app_state.search) {
            any = true;
            let match_range = ind..(ind + match_text.len());
            Self::ensure_splits_at(&mut job.sections, match_range.start);
            Self::ensure_splits_at(&mut job.sections, match_range.end);
            for section in &mut job.sections {
                assert!(
                    match_range.end <= section.byte_range.start
                        || section.byte_range.end <= match_range.start
                        || match_range.start <= section.byte_range.start
                            && match_range.end >= section.byte_range.end
                );
                if match_range.contains(&section.byte_range.start) {
                    section.format.background = HLSEARCH;
                }
            }
        }
        any
    }

    fn ensure_splits_at(sections: &mut Vec<LayoutSection>, index: usize) {
        let mut i = 0;
        while i < sections.len() {
            let byte_range = sections[i].byte_range.clone();
            if byte_range.contains(&index) && index != byte_range.start && index != byte_range.end {
                sections.insert(i + 1, sections[i].clone());
                sections[i].byte_range.end = index;
                sections[i + 1].byte_range.start = index;
            }
            i += 1;
        }
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

trait Appendable {
    fn append(&mut self, text: &str, leading_space: f32, format: TextFormat);
}

impl Appendable for String {
    fn append(&mut self, text: &str, _leading_space: f32, _format: TextFormat) {
        self.push_str(text);
    }
}

impl Appendable for LayoutJob {
    fn append(&mut self, text: &str, leading_space: f32, format: TextFormat) {
        self.append(text, leading_space, format);
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

impl ParsedMessage<'_> {
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
