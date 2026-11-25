use bumpalo::Bump;
use eframe::{
    CreationContext, NativeOptions,
    egui::{CentralPanel, Color32, FontId, Frame, Key, Modifiers, Panel, TextFormat, Ui},
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

use crate::{
    filter::Filter,
    parser::{JsonMap, Span},
};

mod filter;
mod message;
mod parser;
mod scroller;

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
    messages: Vec<message::Message<'b>>,
    messages_reader: Receiver<message::Message<'b>>,
    ui_has_been_notified: Arc<AtomicBool>,
    start_time: Option<Instant>,
    end_time: Option<Instant>,
    state: AppState,
}

#[derive(Default)]
struct AppState {
    scroll_value: scroller::Value,
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

    fn recv_message(&mut self, message: message::Message<'b>) {
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
            scroller::mouse_input(input, &mut self.state.scroll_value);
            if self.state.entering_search_text {
                if input.consume_key(Modifiers::NONE, Key::Escape) {
                    self.state.entering_search_text = false;
                }
            } else {
                scroller::key_input(
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
                        && let Some(index) = self.prev_search(self.state.scroll_value.index())
                    {
                        self.state.scroll_value = scroller::Value::at_index(index);
                    }
                    if input.consume_key(Modifiers::NONE, Key::N)
                        && let Some(index) = self.next_search(
                            (self.state.scroll_value.index() + 1) % self.messages.len(),
                        )
                    {
                        self.state.scroll_value = scroller::Value::at_index(index);
                    }
                }
            }
        });
        scroller::clamp(&mut self.state.scroll_value, self.messages.len());
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
            scroller::big(
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
            && let Some(index) = self.next_search(self.state.scroll_value.index())
        {
            self.state.scroll_value = scroller::Value::at_index(index);
        }
    }
}

fn read_lines<'b: 'scope, 'scope, 'env>(
    bump: &'b mut Bump,
    scope: &'scope std::thread::Scope<'scope, 'env>,
    data_source: DataSource,
    notify_ui_thread: impl Fn() + Send + 'static,
) -> Receiver<message::Message<'b>> {
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
    send: Sender<message::Message<'b>>,
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
        let message = message::Message::new(bump, &line, i, &mut parent_stack).unwrap();
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
