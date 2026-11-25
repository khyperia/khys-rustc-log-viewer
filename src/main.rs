use bumpalo::Bump;
use eframe::{
    CreationContext, NativeOptions,
    egui::{CentralPanel, Color32, Frame, Key, Modifiers, Panel, Ui},
    egui_wgpu,
};
use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Instant,
};

use crate::{filter::Filter, message::HopMessageKind, parser::DataSource};

mod cli;
mod filter;
mod message;
mod parser;
mod scroller;

fn main() -> anyhow::Result<()> {
    let Some(data_source) = cli::parse_cmdline() else {
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
        let (send, messages_reader) = mpsc::channel();
        scope.spawn(move || {
            data_source.parse(
                bump,
                move || {
                    if !atomic_bool.swap(true, Ordering::Relaxed) {
                        egui_ctx.request_repaint();
                    }
                },
                send,
            )
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
