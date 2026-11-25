use super::{
    AppState, COLON_COLON, FIELD_KEYS, FIELD_VALUES, FILENAME, HLSEARCH, HopMessageKind, INDENT,
    LOGPARSE_CONFIG, SPAN, SPAN_KEYS, SPAN_NAME, TARGET, TIMESTAMP, dict_matches_search,
    filter::{Filter, FilterKind},
    log_level_color, logparse_color,
    parser::{JsonMap, LinkedList, ParsedMessage},
    scroller, span_matches_search, text_format, text_format_color,
};
use bumpalo::Bump;
use eframe::egui::{
    Color32, Frame, TextFormat, Ui, UiBuilder,
    text::{LayoutJob, LayoutSection},
};
use std::num::NonZeroUsize;

pub struct Message<'b> {
    pub parsed: ParsedMessage<'b>,
    // Box, to keep the size of the struct down
    #[allow(clippy::box_collection)]
    logparsed_message: Option<Box<Vec<logparse::Span<'b>>>>,
    pub parent: Option<usize>,
    // if this is an Enter message, this is the index of the corresponding Exit
    // NonZeroUsize to get a niche opti to keep the size down
    pub exit: Option<NonZeroUsize>,
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
    pub fn new(
        bump: &'b Bump,
        line: &str,
        index: usize,
        parent_stack: &mut Vec<usize>,
    ) -> anyhow::Result<Self> {
        let parsed = crate::parser::custom_parse(bump, line)?;
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

    pub fn is_displayed(&self, messages: &[Message], app_state: &AppState) -> bool {
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

    pub fn ui_outer(
        &mut self,
        parent: Option<&mut Message>,
        app_state: &mut AppState,
        ui: &mut Ui,
    ) {
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
                app_state.scroll_value = scroller::Value::at_index(exit.get());
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
                app_state.scroll_value = scroller::Value::at_index(parent);
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

    pub fn build_text(&self, app_state: &AppState, job: &mut impl Appendable) {
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

    pub fn matches_search(&self, search: &str) -> bool {
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

pub trait Appendable {
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
