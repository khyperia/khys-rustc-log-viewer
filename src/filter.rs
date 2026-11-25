use super::message::Message;
use eframe::egui::{ComboBox, Ui};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::PathBuf,
};

#[derive(Serialize, Deserialize)]
pub struct Filter {
    pub kind: FilterKind,
    pub filter: String,
    pub exclude: bool,
}

impl Filter {
    pub fn new() -> Self {
        Self {
            kind: FilterKind::Target,
            filter: String::new(),
            exclude: true,
        }
    }

    fn file() -> Option<PathBuf> {
        dirs::config_dir().map(|mut v| {
            v.push("khys-rustc-log-viewer");
            v.push("filters.json");
            v
        })
    }

    pub fn from_file() -> Result<Vec<Self>, String> {
        let config_path = Self::file().ok_or_else(|| "no config dir available".to_string())?;
        let file = File::open(config_path).map_err(|a| {
            if a.kind() == std::io::ErrorKind::NotFound {
                String::new()
            } else {
                a.to_string()
            }
        })?;
        let reader = BufReader::new(file);
        serde_json::from_reader(reader).map_err(|e| e.to_string())
    }

    pub fn to_file(v: &[Self]) -> Result<(), String> {
        let config_path = Self::file().ok_or_else(|| "no config dir available".to_string())?;
        if v.is_empty() {
            match std::fs::remove_file(&config_path) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.to_string()),
            }
            if let Some(parent) = config_path.parent() {
                match std::fs::remove_dir(parent) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => (),
                    Err(e) => return Err(e.to_string()),
                }
            }
            Ok(())
        } else {
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let file = File::create(config_path).map_err(|e| e.to_string())?;
            let writer = BufWriter::new(file);
            serde_json::to_writer(writer, v).map_err(|e| e.to_string())?;
            Ok(())
        }
    }

    pub fn matches(&self, message: &Message) -> bool {
        if self.filter.is_empty() {
            true
        } else {
            self.exclude ^ self.matches_raw(message)
        }
    }

    fn matches_raw(&self, message: &Message) -> bool {
        self.kind.run(message, |v| v.contains(&self.filter))
    }

    pub fn ui(&mut self, id_salt: impl std::hash::Hash, ui: &mut Ui) -> bool {
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

#[derive(Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub enum FilterKind {
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
    fn run(self, message: &Message, mut visit: impl FnMut(&str) -> bool) -> bool {
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

    fn name(self) -> &'static str {
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
