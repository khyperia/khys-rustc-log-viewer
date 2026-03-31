use eframe::{
    CreationContext,
    egui::{
        CentralPanel, Color32, ComboBox, FontId, Frame, InputState, Key, Modifiers, Panel,
        TextFormat, Ui, UiBuilder, text::LayoutJob,
    },
};
use std::{
    borrow::Cow,
    collections::HashMap,
    error::Error,
    fmt::{Debug, Display},
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    str::{Chars, FromStr},
    time::Instant,
};
use yoke::Yoke;

const GREY: Color32 = Color32::from_rgb(142, 142, 142);
const WHITE: Color32 = Color32::from_rgb(216, 216, 216);
const BLUE: Color32 = Color32::from_rgb(106, 159, 181);
const PURPLE: Color32 = Color32::from_rgb(170, 117, 159);
const GREEN: Color32 = Color32::from_rgb(144, 169, 89);
const RED: Color32 = Color32::from_rgb(172, 66, 68);
const CYAN: Color32 = Color32::from_rgb(117, 181, 170);
const YELLOW: Color32 = Color32::from_rgb(244, 191, 117);
const HLSEARCH: Color32 = Color32::from_rgb(0, 92, 128);
const INDENT: f32 = 8.0;

fn main() -> eframe::Result {
    env_logger::init();
    eframe::run_native(
        "Viewlog",
        Default::default(),
        Box::new(|cc| Ok(Box::new(App::new(cc)?))),
    )
}

struct App {
    messages: Vec<Message>,
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

    fn vdict_matches_search(&self, list: &[HashMap<Cow<str>, SpanValue>]) -> bool {
        if self.search.is_empty() {
            false
        } else {
            list.iter()
                .any(|map| dict_matches_search(map, &self.search))
        }
    }
}

impl App {
    fn new(_cc: &CreationContext) -> anyhow::Result<Self> {
        let start = Instant::now();
        let messages = read_lines(Path::new("/home/khyperia/rustc-log.txt"), |s| {
            custom_parse(s)
        })?;
        let end = Instant::now();
        println!("{:?}", end - start);
        Ok(App {
            messages,
            scroll_value: Default::default(),
            state: AppState::new(),
        })
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
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
            ui.add_space(5.0);
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
                if let SpanValue::String(v) = v {
                    visit(v)
                } else {
                    false
                }
            }),
            Self::Target => visit(&message.parsed().target),
            Self::Filename => visit(&message.parsed().filename),
            Self::SpanName => message.parsed().span.get("name").is_some_and(|n| {
                if let SpanValue::String(n) = n {
                    visit(n)
                } else {
                    false
                }
            }),
            Self::Spans => message.parsed().spans.iter().any(|n| {
                n.iter().any(|(k, v)| {
                    visit(k);
                    if let SpanValue::String(v) = v {
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

fn read_lines<E: Error + Send + Sync + 'static>(
    path: &Path,
    parse: impl for<'a> Fn(&'a str) -> Result<ParsedMessage<'a>, E>,
) -> anyhow::Result<Vec<Message>> {
    let file = File::open(path).unwrap();
    let reader = BufReader::new(file);
    let mut parent_stack = vec![];
    let parse = &parse;
    reader
        .lines()
        .enumerate()
        //.take(100000)
        .map(move |(i, s)| Message::new(s?, i, &mut parent_stack, parse))
        .collect()
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

struct Message {
    yoke: Yoke<ParsedMessage<'static>, String>,
    parent: Option<usize>,
    indent: usize,
    state: MessageState,
}

#[derive(Default)]
struct MessageState {
    hide_children: bool,
    display_filename: bool,
    display_spans: bool,
    display_raw_json: bool,
}

impl Message {
    fn new<E: Error + Send + Sync + 'static>(
        line: String,
        index: usize,
        parent_stack: &mut Vec<usize>,
        parse: impl for<'a> FnOnce(&'a str) -> Result<ParsedMessage<'a>, E>,
    ) -> anyhow::Result<Self> {
        let yoke = Yoke::try_attach_to_cart(line, |l| parse(l))?;
        let parsed: &ParsedMessage = yoke.get();
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
            yoke,
            parent,
            indent: self_indent,
            state: Default::default(),
        })
    }

    fn original(&self) -> &str {
        self.yoke.backing_cart()
    }

    fn parsed<'a: 'b, 'b>(&'a self) -> &'a ParsedMessage<'b> {
        self.yoke.get()
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
        if self.state.display_raw_json {
            self.raw_json(&mut job);
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
            ui.checkbox(&mut self.state.display_raw_json, "raw json");
            if ui.button("exclude target filter").clicked() {
                app_state.filters.push(Filter {
                    kind: FilterKind::Target,
                    filter: self.parsed().target.to_string(),
                    exclude: true,
                });
            }
        });
    }

    fn main_text(&self, job: &mut StrBuilder) {
        let barsed = self.parsed();
        if job.app_state.timestamps || job.app_state.matches_search(&barsed.timestamp) {
            job.append(&barsed.timestamp, 0.0, text_format_color(GREY));
            job.append(" ", 0.0, text_format_color(GREY));
        }
        if job.app_state.log_levels || job.app_state.matches_search(&barsed.level) {
            let color = match &*barsed.level {
                "TRACE" => PURPLE,
                "DEBUG" => BLUE,
                "INFO" => GREEN,
                "WARN" => YELLOW, // TODO: is WARN the right name for this? (i've never seen it)
                _ => RED,
            };
            job.append(&barsed.level, 0.0, text_format_color(color));
            job.append(" ", 0.0, text_format_color(color));
        }
        let target_displayed =
            job.app_state.targets || job.app_state.matches_search(&barsed.target);
        if target_displayed {
            job.append(&barsed.target, 0.0, text_format_color(CYAN));
        }
        if let Some(SpanValue::String(name)) = barsed.span.get("name") {
            if target_displayed {
                job.append("::", 0.0, text_format_color(WHITE));
            }
            job.append(name, 0.0, text_format_color(GREEN));
        } else if !target_displayed {
            // TODO: DRY
            job.append(&barsed.target, 0.0, text_format_color(CYAN));
        }
        self.dict(job, INDENT, GREY, &barsed.fields);
        if barsed.fields.len() == 1 && barsed.hop_message().is_some() {
            // enter/exit messages get span
            self.dict(job, INDENT, YELLOW, &barsed.span);
        }
    }

    fn filename(&self, job: &mut StrBuilder) {
        let parsed = self.parsed();
        job.append("\n", 0.0, text_format());
        job.append(&parsed.filename, INDENT, text_format_color(GREY));
        job.append(":", 0.0, text_format_color(GREY));
        job.append(
            &format!("{}", parsed.line_number),
            0.0,
            text_format_color(GREY),
        );
    }

    fn spans(&self, job: &mut StrBuilder) {
        // spans are reversed from the normal stack trace
        for span in self.parsed().spans.iter().rev() {
            job.append("\n", 0.0, text_format());
            let name = match span.get("name") {
                Some(SpanValue::String(name)) => name,
                _ => "---",
            };
            job.append(name, INDENT, text_format_color(GREEN));
            self.dict(job, INDENT * 2.0, GREY, span)
        }
    }

    fn dict(
        &self,
        job: &mut StrBuilder,
        mut indent: f32,
        key_color: Color32,
        map: &HashMap<Cow<str>, SpanValue>,
    ) {
        let total: usize = map
            .iter()
            .map(|(k, v)| {
                k.len()
                    + match v {
                        SpanValue::Bool(false) => 5,
                        SpanValue::Bool(true) => 4,
                        SpanValue::Int(v) => format!("{v}").len(),
                        SpanValue::String(s) => s.len(),
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
            job.append(sep, 0.0, text_format_color(WHITE));
            job.append(key, indent, text_format_color(key_color));
            job.append(": ", 0.0, text_format_color(key_color));
            let value = match value {
                &SpanValue::Bool(v) => {
                    if v {
                        "true"
                    } else {
                        "false"
                    }
                }
                &SpanValue::Int(v) => &format!("{v}"),
                SpanValue::String(cow) => &**cow,
            };
            job.append(value, 0.0, text_format_color(WHITE));
        }
    }

    fn raw_json(&self, job: &mut StrBuilder) {
        job.append("\n", 0.0, text_format());
        job.append(self.original(), 0.0, text_format_color(GREY));
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

fn dict_matches_search(map: &HashMap<Cow<str>, SpanValue>, search: &str) -> bool {
    map.iter().any(|(k, v)| {
        k.contains(search)
            || match v {
                SpanValue::Bool(_) => false,
                SpanValue::Int(_) => false,
                SpanValue::String(s) => s.contains(search),
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
struct ParsedMessage<'a> {
    timestamp: Cow<'a, str>,
    level: Cow<'a, str>,
    fields: HashMap<Cow<'a, str>, SpanValue<'a>>,
    target: Cow<'a, str>,
    filename: Cow<'a, str>,
    line_number: u64,
    span: HashMap<Cow<'a, str>, SpanValue<'a>>,
    spans: Vec<HashMap<Cow<'a, str>, SpanValue<'a>>>,
}

impl ParsedMessage<'_> {
    fn hop_message(&self) -> Option<HopMessageKind> {
        self.fields.get("message").and_then(|v| {
            if let SpanValue::String(v) = v {
                match &**v {
                    "enter" => Some(HopMessageKind::Enter),
                    "exit" => Some(HopMessageKind::Exit),
                    _ => None,
                }
            } else {
                None
            }
        })
    }
}

enum SpanValue<'a> {
    Bool(bool),
    Int(i64),
    String(Cow<'a, str>),
}

#[derive(Debug)]
struct ParseError;

impl Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("parse error")
    }
}

impl Error for ParseError {}

fn custom_parse<'a>(s: &'a str) -> Result<ParsedMessage<'a>, ParseError> {
    custom_parse2(&mut s.chars()).map_err(|()| ParseError)
}

fn custom_parse2<'a>(iter: &mut Chars<'a>) -> Result<ParsedMessage<'a>, ()> {
    let mut result = ParsedMessage::default();

    expect(iter, '{')?;

    loop {
        let key = parse_string(iter)?;
        expect(iter, ':')?;
        match &*key {
            "timestamp" => result.timestamp = parse_string(iter)?,
            "level" => result.level = parse_string(iter)?,
            "fields" => parse_dict(iter, &mut result.fields)?,
            "target" => result.target = parse_string(iter)?,
            "filename" => result.filename = parse_string(iter)?,
            "line_number" => result.line_number = parse_number(iter)?,
            "span" => parse_dict(iter, &mut result.span)?,
            "spans" => parse_dicts(iter, &mut result.spans)?,
            _ => panic!("unexpected key {key}"),
        }
        match iter.next().ok_or(())? {
            '}' => break Ok(result),
            ',' => continue,
            ch => panic!("unexpected ch {ch}"),
        }
    }
}

fn parse_dicts<'a>(
    iter: &mut Chars<'a>,
    result: &mut Vec<HashMap<Cow<'a, str>, SpanValue<'a>>>,
) -> Result<(), ()> {
    expect(iter, '[')?;
    let mut iter_clone = iter.clone();
    if iter_clone.next() == Some(']') {
        *iter = iter_clone;
        return Ok(());
    }
    loop {
        let mut single = Default::default();
        parse_dict(iter, &mut single)?;
        result.push(single);
        match iter.next().ok_or(())? {
            ']' => break Ok(()),
            ',' => continue,
            _ => break Err(()),
        }
    }
}

fn parse_dict<'a>(
    iter: &mut Chars<'a>,
    result: &mut HashMap<Cow<'a, str>, SpanValue<'a>>,
) -> Result<(), ()> {
    expect(iter, '{')?;
    loop {
        let key = parse_string(iter)?;
        expect(iter, ':')?;
        let start = iter.as_str();
        let value = match iter.next().ok_or(())? {
            '"' => SpanValue::String(parse_string_after_quote(iter)?),
            't' => {
                expect(iter, 'r')?;
                expect(iter, 'u')?;
                expect(iter, 'e')?;
                SpanValue::Bool(true)
            }
            'f' => {
                expect(iter, 'a')?;
                expect(iter, 'l')?;
                expect(iter, 's')?;
                expect(iter, 'e')?;
                SpanValue::Bool(false)
            }
            ch if ch.is_ascii_digit() => SpanValue::Int(parse_number_after_digit(start, iter)?),
            ch => panic!("unexpected ch {ch}"),
        };
        result.insert(key, value);
        match iter.next().ok_or(())? {
            '}' => break Ok(()),
            ',' => continue,
            ch => panic!("unexpected ch {ch}"),
        }
    }
}

fn parse_string<'a>(iter: &mut Chars<'a>) -> Result<Cow<'a, str>, ()> {
    expect(iter, '"')?;
    parse_string_after_quote(iter)
}

fn parse_string_after_quote<'a>(iter: &mut Chars<'a>) -> Result<Cow<'a, str>, ()> {
    let str_begin = iter.as_str();
    let Some(quot) = str_begin.find('"') else {
        return Err(());
    };
    let str_to_quot = &str_begin[..quot];
    if let Some(_backslash) = str_to_quot.find('\\') {
    } else {
        *iter = str_begin[(quot + 1)..].chars();
        return Ok(Cow::Borrowed(str_to_quot));
    }
    // approx. 1.43% of strings had an escape in the dataset I used to test
    loop {
        let ch = iter.next().ok_or(())?;
        if ch == '"' {
            let str_end = iter.as_str();
            let len = str_begin.len() - (str_end.len() + 1);
            break Ok(Cow::Borrowed(&str_begin[..len]));
        } else if ch == '\\' {
            let Some(_escaped_ch) = iter.next() else {
                panic!("unexpected eof: {ch}");
            };
        }
    }
}

fn parse_number<'a, T: FromStr>(iter: &mut Chars<'a>) -> Result<T, ()>
where
    <T as FromStr>::Err: Debug,
{
    let start = iter.as_str();
    let ch = iter.next().ok_or(())?;
    if ch.is_ascii_digit() {
        parse_number_after_digit(start, iter)
    } else {
        Err(())
    }
}

fn parse_number_after_digit<'a, T: FromStr>(start: &'a str, iter: &mut Chars<'a>) -> Result<T, ()>
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
    T::from_str(slice).map_err(|_| ())
}

fn expect<'a>(iter: &mut Chars<'a>, ch: char) -> Result<(), ()> {
    if let Some(got) = iter.next() {
        if got == ch { Ok(()) } else { Err(()) }
    } else {
        Err(())
    }
}
