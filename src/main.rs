use eframe::{
    CreationContext,
    egui::{
        CentralPanel, Color32, ComboBox, FontId, Frame, Key, Modifiers, Panel, TextFormat, Ui,
        UiBuilder, text::LayoutJob,
    },
};
use std::{
    borrow::Cow,
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};
use yoke::Yoke;

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
    filters: Vec<Filter>,
}

impl App {
    fn new(_cc: &CreationContext) -> anyhow::Result<Self> {
        let messages = read_lines(Path::new("/home/khyperia/rustc-log.txt"))?;
        Ok(App {
            messages,
            scroll_value: Default::default(),
            state: Default::default(),
        })
    }

    // silly nit: index is *inclusive* here
    fn next_search(&self, index: usize) -> Option<usize> {
        (index..self.messages.len())
            .chain(0..index)
            .find(|&index| self.messages[index].matches_search(&self.state.search))
    }

    // silly nit: index is *exclusive* here
    fn prev_search(&self, index: usize) -> Option<usize> {
        (index..self.messages.len())
            .chain(0..index)
            .rev()
            .find(|&index| self.messages[index].matches_search(&self.state.search))
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let mut opened_search = false;
        ui.input_mut(|input| {
            if self.state.entering_search_text {
                if input.consume_key(Modifiers::NONE, Key::Escape) {
                    self.state.entering_search_text = false;
                }
            } else {
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
                if input.consume_key(Modifiers::NONE, Key::Slash) {
                    self.state.entering_search_text = true;
                    opened_search = true;
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
            big_scroller(ui, &mut self.scroll_value, |ui, index| {
                if index < self.messages.len() {
                    if self.messages[index].is_displayed(&self.messages, &self.state) {
                        self.messages[index].ui_outer(&mut self.state, ui);
                    }
                    Some(())
                } else {
                    None
                }
            });
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

fn big_scroller(
    ui: &mut Ui,
    value: &mut ScrollValue,
    mut draw: impl FnMut(&mut Ui, usize) -> Option<()>,
) {
    ui.input_mut(|input| {
        let scroll_mult = -4.0; // inverted?
        value.pixel_offset += input.smooth_scroll_delta.y * scroll_mult;
        // if input.smooth_scroll_delta.y != 0.0 {
        //     println!("scroll input: {} {}", value.index, value.pixel_offset);
        // }
        input.smooth_scroll_delta.y = 0.0;
        if input.consume_key(Modifiers::NONE, Key::J) {
            value.pixel_offset += 1.0;
        }
        if input.consume_key(Modifiers::NONE, Key::K) {
            value.pixel_offset -= 1.0;
        }
    });

    ui.scope(|ui| {
        let max_rect = ui.max_rect();
        let absolute_begin = ui.next_widget_position().y;
        ui.add_space(-value.pixel_offset);
        let mut index = value.index;
        ui.skip_ahead_auto_ids(index);
        loop {
            let begin = ui.next_widget_position().y;
            let Some(()) = draw(ui, index) else { break };
            let end = ui.next_widget_position().y;
            if end > max_rect.bottom() {
                break;
            }
            if end < absolute_begin {
                value.index += 1;
                let size = end - begin;
                value.pixel_offset -= size;
                // println!("next scroll: {} {}", value.index, value.pixel_offset);
            }
            index += 1;
        }
    });

    // this messes up the drawing of the main content, idk why, so put it after
    if value.pixel_offset < 0.0 && value.index > 0 {
        ui.scope_builder(UiBuilder::new().sizing_pass().invisible(), |ui| {
            while value.pixel_offset < 0.0 && value.index > 0 {
                let begin = ui.next_widget_position().y;
                let Some(()) = draw(ui, value.index - 1) else {
                    break;
                };
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
                    let kinds = [FilterKind::Target];

                    for kind in kinds {
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
    Target,
}

impl FilterKind {
    fn run(&self, message: &Message, mut visit: impl FnMut(&str) -> bool) -> bool {
        match self {
            FilterKind::Target => visit(&message.parsed().target),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            FilterKind::Target => "target",
        }
    }
}

fn read_lines(path: &Path) -> anyhow::Result<Vec<Message>> {
    let file = File::open(path).unwrap();
    let reader = BufReader::new(file);
    let mut parent_stack = vec![];
    reader
        .lines()
        .enumerate()
        .take(1000)
        .map(|(i, s)| Message::new(s?, i, &mut parent_stack))
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

const GREY: Color32 = Color32::from_rgb(142, 142, 142);
const WHITE: Color32 = Color32::from_rgb(216, 216, 216);
const BLUE: Color32 = Color32::from_rgb(106, 159, 181);
const PURPLE: Color32 = Color32::from_rgb(170, 117, 159);
const GREEN: Color32 = Color32::from_rgb(144, 169, 89);
//const RED: Color32 = Color32::from_rgb(172, 66, 68);
const CYAN: Color32 = Color32::from_rgb(117, 181, 170);
//const YELLOW: Color32 = Color32::from_rgb(244, 191, 117);
const HLSEARCH: Color32 = Color32::from_rgb(0, 92, 128);

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
    fn new(line: String, index: usize, parent_stack: &mut Vec<usize>) -> anyhow::Result<Self> {
        let yoke = Yoke::try_attach_to_cart(line, |l| facet_json::from_str_borrowed(l))?;
        let parsed: &ParsedMessage = yoke.get();
        let msg = parsed.fields.get("message").map(|v| v as &str);
        if msg == Some("exit") {
            parent_stack.pop();
        }
        let self_indent = parent_stack.len();
        let parent = parent_stack.last().cloned();
        if msg == Some("enter") {
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

    // fn parsed_mut<F, R>(&mut self, func: F) -> R
    // where
    //     F: 'static + for<'b> FnOnce(&'b mut ParsedMessage<'_>) -> R,
    //     R: 'static,
    // {
    //     self.yoke.with_mut_return(func)
    // }

    fn hop_message(&self) -> Option<&str> {
        self.parsed().fields.get("message").map(|v| &**v)
    }

    // fn hop_kind(&self) -> HopKind {
    //     match (self.parsed().hop_index, self.hop_message()) {
    //         (Some(idx), Some("enter")) => HopKind::Enter(idx),
    //         (Some(idx), Some("exit")) => HopKind::Exit(idx),
    //         _ => HopKind::None,
    //     }
    // }

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

    fn ui_outer(&mut self, app_state: &mut AppState, ui: &mut Ui) {
        let mut child_rect = ui.available_rect_before_wrap();
        child_rect.min.x += ui.spacing().indent * self.indent as f32;
        ui.scope_builder(UiBuilder::new().max_rect(child_rect), |ui| {
            self.ui(app_state, ui);
        });
    }

    fn ui(&mut self, app_state: &mut AppState, ui: &mut Ui) {
        let mut job = LayoutJob::default();
        self.main_text(&mut job);
        if self.state.display_filename {
            self.filename(&mut job);
        }
        if self.state.display_spans {
            self.spans(&mut job);
        }
        if self.state.display_raw_json {
            self.raw_json(&mut job);
        }
        let mut found_search = false;
        if !app_state.search.is_empty() {
            self.search(&mut job, &mut found_search, app_state);
        }
        let rsp = if !app_state.search.is_empty()
            && !found_search
            && self.matches_search(&app_state.search)
        {
            // fallback to highlight the whole message if we're not displaying the matching text
            Frame::NONE
                .fill(HLSEARCH)
                .show(ui, |ui| ui.label(job))
                .inner
        } else {
            ui.label(job)
        };
        if found_search && ui.clip_rect().intersects(rsp.rect) {
            app_state.search_onscreen = true;
        }
        rsp.context_menu(|ui| {
            if let Some("enter") = self.hop_message() {
                ui.checkbox(&mut self.state.hide_children, "hide children");
            }
            ui.checkbox(&mut self.state.display_filename, "filename");
            if self.parsed().spans.is_some() {
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

    fn main_text(&self, job: &mut LayoutJob) {
        let barsed = self.parsed();
        level(job, 0.0, &barsed.level);
        job.append(
            &barsed.target,
            0.0,
            TextFormat {
                color: CYAN,
                ..text_format()
            },
        );
        if let Some(span) = &barsed.span
            && let Some(SpanValue::String(name)) = span.get("name")
        {
            job.append(
                "::",
                0.0,
                TextFormat {
                    color: WHITE,
                    ..text_format()
                },
            );
            job.append(
                name,
                0.0,
                TextFormat {
                    color: GREEN,
                    ..text_format()
                },
            );
        }
        self.dict(job, 8.0, &barsed.fields);
    }

    fn filename(&self, job: &mut LayoutJob) {
        let parsed = self.parsed();
        job.append("\n", 0.0, text_format());
        job.append(&parsed.filename, 8.0, text_format_color(GREY));
        job.append(":", 0.0, text_format_color(GREY));
        // TODO: optimize str allocation here
        job.append(
            &format!("{}", parsed.line_number),
            0.0,
            text_format_color(GREY),
        );
    }

    fn spans(&self, job: &mut LayoutJob) {
        if let Some(spans) = &self.parsed().spans {
            for span in spans {
                let mut indent = Some(8.0);
                job.append("\n", 0.0, text_format());
                if let Some(SpanValue::String(name)) = span.get("name") {
                    job.append(
                        name,
                        indent.take().unwrap_or(0.0),
                        TextFormat {
                            color: GREEN,
                            ..text_format()
                        },
                    );
                }
                self.dict(job, indent.take().unwrap_or(0.0), span)
            }
        }
    }

    fn dict(&self, job: &mut LayoutJob, mut indent: f32, map: &HashMap<Cow<str>, impl CowStrable>) {
        let total: usize = map
            .iter()
            .map(|(k, v)| k.len() + v.to_cow_str().len())
            .sum();
        let sep = if total > 100 {
            "\n"
        } else {
            indent = 0.0;
            " "
        };
        for (key, value) in map {
            job.append(sep, 0.0, text_format_color(WHITE));
            job.append(key, indent, text_format_color(GREY));
            job.append(": ", 0.0, text_format_color(GREY));
            job.append(&value.to_cow_str(), 0.0, text_format_color(WHITE));
        }
    }

    fn raw_json(&self, job: &mut LayoutJob) {
        job.append("\n", 0.0, text_format());
        job.append(
            self.original(),
            0.0,
            TextFormat {
                color: GREY,
                ..text_format()
            },
        );
    }

    fn search(&self, job: &mut LayoutJob, found_search: &mut bool, app_state: &AppState) {
        let section_idx = job.sections.iter().enumerate().find_map(|(i, v)| {
            job.text[v.byte_range.clone()]
                .find(&app_state.search)
                .map(|index| (i, v.byte_range.start + index))
        });
        if let Some((section_idx, str_index)) = section_idx {
            let mut section = job.sections.remove(section_idx);
            let mut prefix = section.clone();
            let mut suffix = section.clone();
            let endpoint = str_index + app_state.search.len();
            let old_start = section.byte_range.start;
            let old_end = section.byte_range.end;
            prefix.byte_range = old_start..str_index;
            section.byte_range = str_index..endpoint;
            suffix.byte_range = endpoint..old_end;
            section.format.background = HLSEARCH;
            job.sections.insert(section_idx, suffix);
            job.sections.insert(section_idx, section);
            job.sections.insert(section_idx, prefix);
            *found_search = true;
        }
    }

    fn matches_search(&self, search: &str) -> bool {
        fn sp(map: &HashMap<Cow<'_, str>, SpanValue>, search: &str) -> bool {
            map.iter().any(|(k, v)| {
                k.contains(search)
                    || match v {
                        SpanValue::Bool(_) => false,
                        SpanValue::Int(_) => false,
                        SpanValue::String(s) => s.contains(search),
                    }
            })
        }
        let parsed = self.parsed();
        parsed.timestamp.contains(search)
            || parsed.target.contains(search)
            || parsed.filename.contains(search)
            || parsed
                .fields
                .iter()
                .any(|(k, v)| k.contains(search) || v.contains(search))
            || parsed.span.as_ref().is_some_and(|m| sp(m, search))
            || parsed
                .spans
                .as_ref()
                .is_some_and(|m| m.iter().any(|m| sp(m, search)))
    }
}

fn level(job: &mut LayoutJob, sp: f32, level: &Level) {
    let (text, color) = match level {
        Level::TRACE => ("TRACE ", PURPLE),
        Level::DEBUG => ("DEBUG ", BLUE),
        Level::INFO => ("INFO ", GREEN),
    };
    let fmt = TextFormat {
        color,
        ..text_format()
    };
    job.append(text, sp, fmt)
}

#[derive(Default, facet::Facet, yoke::Yokeable)]
#[facet(deny_unknown_fields)]
struct ParsedMessage<'a> {
    timestamp: Cow<'a, str>,
    level: Level,
    fields: HashMap<Cow<'a, str>, Cow<'a, str>>,
    target: Cow<'a, str>,
    filename: Cow<'a, str>,
    line_number: u64,
    span: Option<HashMap<Cow<'a, str>, SpanValue<'a>>>,
    spans: Option<Vec<HashMap<Cow<'a, str>, SpanValue<'a>>>>,
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Default, facet::Facet)]
#[repr(C)]
enum Level {
    #[default]
    TRACE,
    DEBUG,
    INFO,
}

trait CowStrable {
    fn to_cow_str(&self) -> Cow<'_, str>;
}

impl CowStrable for Cow<'_, str> {
    fn to_cow_str(&self) -> Cow<'_, str> {
        Cow::Borrowed(&**self)
    }
}

impl CowStrable for SpanValue<'_> {
    fn to_cow_str(&self) -> Cow<'_, str> {
        match self {
            SpanValue::Bool(v) => Cow::Owned(format!("{v}")),
            SpanValue::Int(v) => Cow::Owned(format!("{v}")),
            SpanValue::String(cow) => cow.to_cow_str(),
        }
    }
}

#[derive(facet::Facet)]
#[facet(untagged)]
#[repr(u8)]
enum SpanValue<'a> {
    Bool(bool),
    Int(i64),
    String(Cow<'a, str>),
}
