//! Browser entry point for a real egui SCAS workspace. The native IDE remains
//! separate while platform services are being extracted from its binary.

use crate::ast::SourceSpan;
use crate::document::{self, Document, DocumentFragment, FragmentKind};
use crate::intelligence::SourceIndex;
use crate::lexer::{Scanner, TokenType};
use crate::plot::{self, PlotBounds};
use crate::web_runtime::{MIT_EXAMPLE, STARTER};
use eframe::egui::{
    self,
    text::{LayoutJob, TextFormat},
    Color32, FontData, FontDefinitions, FontFamily, FontId, RichText, TextEdit,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, Worker};

#[path = "bin/scas_gui/theme.rs"]
#[allow(dead_code)]
mod theme;
use theme::Theme;

const DEFAULT_EDITOR_FONT_SIZE: f32 = 27.0;
const MIN_EDITOR_FONT_SIZE: f32 = 9.0;
const MAX_EDITOR_FONT_SIZE: f32 = 32.0;
const MIN_EDITOR_WORKSPACE_WIDTH: f32 = 420.0;
const MIN_PREVIEW_WIDTH: f32 = 220.0;
const MAX_PREVIEW_WIDTH: f32 = 900.0;
const GREEK: &[(&str, &str)] = &[
    ("\\Alpha", "Α"),
    ("\\Beta", "Β"),
    ("\\Gamma", "Γ"),
    ("\\Delta", "Δ"),
    ("\\Epsilon", "Ε"),
    ("\\Zeta", "Ζ"),
    ("\\Eta", "Η"),
    ("\\Theta", "Θ"),
    ("\\Iota", "Ι"),
    ("\\Kappa", "Κ"),
    ("\\Lambda", "Λ"),
    ("\\Mu", "Μ"),
    ("\\Nu", "Ν"),
    ("\\Xi", "Ξ"),
    ("\\Omicron", "Ο"),
    ("\\Pi", "Π"),
    ("\\Rho", "Ρ"),
    ("\\Sigma", "Σ"),
    ("\\Tau", "Τ"),
    ("\\Upsilon", "Υ"),
    ("\\Phi", "Φ"),
    ("\\Chi", "Χ"),
    ("\\Psi", "Ψ"),
    ("\\Omega", "Ω"),
    ("\\alpha", "α"),
    ("\\beta", "β"),
    ("\\gamma", "γ"),
    ("\\delta", "δ"),
    ("\\epsilon", "ε"),
    ("\\zeta", "ζ"),
    ("\\eta", "η"),
    ("\\theta", "θ"),
    ("\\iota", "ι"),
    ("\\kappa", "κ"),
    ("\\lambda", "λ"),
    ("\\mu", "μ"),
    ("\\nu", "ν"),
    ("\\xi", "ξ"),
    ("\\omicron", "ο"),
    ("\\pi", "π"),
    ("\\rho", "ρ"),
    ("\\sigma", "σ"),
    ("\\tau", "τ"),
    ("\\phi", "φ"),
    ("\\chi", "χ"),
    ("\\psi", "ψ"),
    ("\\omega", "ω"),
    ("\\varepsilon", "ϵ"),
    ("\\vartheta", "ϑ"),
    ("\\varpi", "ϖ"),
    ("\\varrho", "ϱ"),
    ("\\varsigma", "ς"),
    ("\\varphi", "ϕ"),
];

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = scasDownload)]
    fn download_file(name: &str, content: &str, mime: &str);
    #[wasm_bindgen(js_namespace = window, js_name = scasImport)]
    fn import_file();
    #[wasm_bindgen(js_namespace = window, js_name = scasImportTheme)]
    fn import_theme_file();
    #[wasm_bindgen(js_namespace = window, js_name = scasImportFont)]
    fn import_font_file();
    #[wasm_bindgen(js_namespace = window, js_name = scasPrintDocument)]
    fn print_document();
    #[wasm_bindgen(js_namespace = window, js_name = scasSetPreviewTheme)]
    fn set_preview_theme(
        panel: &str,
        editor: &str,
        text: &str,
        muted: &str,
        accent: &str,
        selection: &str,
    );
    #[wasm_bindgen(js_namespace = window, js_name = scasPromptInput)]
    fn prompt_input(message: &str) -> Option<String>;
    #[wasm_bindgen(catch, js_namespace = window, js_name = scasCreateWorker)]
    fn create_worker() -> Result<Worker, JsValue>;
    #[wasm_bindgen(js_namespace = window, js_name = scasShowPreview)]
    fn show_preview(
        fragments: &str,
        tex: &str,
        pending: bool,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        pixels_per_point: f32,
        resizable: bool,
    );
    #[wasm_bindgen(js_namespace = window, js_name = scasHidePreview)]
    fn hide_preview();
}

#[wasm_bindgen]
pub fn worker_evaluate(request: &str) -> String {
    crate::web_runtime::process_request(request)
}

#[wasm_bindgen]
pub struct WebHandle {
    runner: eframe::WebRunner,
}

#[wasm_bindgen]
impl WebHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            runner: eframe::WebRunner::new(),
        }
    }

    pub async fn start(&self, canvas_id: &str) -> Result<(), JsValue> {
        self.runner
            .start(
                canvas_id,
                eframe::WebOptions::default(),
                Box::new(|cc| {
                    configure_context(&cc.egui_ctx);
                    Ok(Box::new(WebApp::new(cc.egui_ctx.clone())))
                }),
            )
            .await
    }

    pub fn load_source(&self, source: String) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.load_source(source);
        }
    }

    pub fn import_data_file(&self, name: String, content: String) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.import_data_file(name, content);
        }
    }

    pub fn import_theme_source(&self, name: String, source: String) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.import_theme(name, source);
        }
    }

    pub fn import_font(&self, name: String, bytes: Vec<u8>) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.install_font(name, bytes);
        }
    }

    pub fn close_preview(&self) -> bool {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.preview_open = false;
            app.context.request_repaint();
            true
        } else {
            false
        }
    }

    pub fn set_preview_width(&self, width: f32) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.preview_width = Some(width.clamp(MIN_PREVIEW_WIDTH, MAX_PREVIEW_WIDTH));
            app.context.request_repaint();
        }
    }

    pub fn reset_preview_width(&self) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.preview_width = None;
            app.context.request_repaint();
        }
    }
}

impl Default for WebHandle {
    fn default() -> Self {
        Self::new()
    }
}

fn web_fonts(custom: Option<(String, Vec<u8>)>) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "JetBrains Mono".to_owned(),
        FontData::from_static(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")),
    );
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrains Mono".to_owned());
    fonts.font_data.insert(
        "VS Code Codicons".to_owned(),
        FontData::from_static(include_bytes!("../assets/icons/codicon.ttf")),
    );
    fonts.families.insert(
        FontFamily::Name("VS Code Codicons".into()),
        vec!["VS Code Codicons".to_owned()],
    );
    if let Some((name, bytes)) = custom {
        fonts
            .font_data
            .insert(name.clone(), FontData::from_owned(bytes));
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, name);
    }
    fonts
}

fn configure_context(context: &egui::Context) {
    context.set_fonts(web_fonts(None));
    Theme::bundled_themes()[0].apply(context);
}

fn preview_widths(screen_width: f32, explorer_width: f32) -> (f32, f32) {
    let usable_width = (screen_width - 48.0 - explorer_width).max(0.0);
    let maximum =
        (usable_width - MIN_EDITOR_WORKSPACE_WIDTH).clamp(MIN_PREVIEW_WIDTH, MAX_PREVIEW_WIDTH);
    let default = (usable_width * 0.46).clamp(MIN_PREVIEW_WIDTH, maximum);
    (default, maximum)
}

fn line_column_at_char(source: &str, char_index: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for character in source.chars().take(char_index) {
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn char_to_byte(source: &str, char_index: usize) -> usize {
    source
        .char_indices()
        .nth(char_index)
        .map_or(source.len(), |(byte, _)| byte)
}

fn highlight_source(source: &str, font: FontId, theme: &Theme) -> LayoutJob {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    let mut spans = tokens
        .iter()
        .filter_map(|token| {
            use TokenType::*;
            let color = match token.token_type {
                Eof => return None,
                Let | Fn | If | Else | For | While | Break | Continue | Return | Print | And
                | Or | Xor | True | False | Nil | Exit | Help | Clear => theme.syntax.keyword.0,
                Number => theme.syntax.number.0,
                Str => theme.syntax.string.0,
                DocComment => theme.syntax.doc_comment.0,
                Identifier => theme.syntax.identifier.0,
                RenderLeftBrace | RenderRightBrace | RenderLeftBracket | RenderRightBracket => {
                    theme.syntax.render_marker.0
                }
                LeftParen | RightParen | LeftBracket | RightBracket | LeftBrace | RightBrace
                | Comma | Dot | Colon | Semicolon => theme.syntax.delimiter.0,
                _ => theme.syntax.operator.0,
            };
            Some((token.byte_start, token.byte_end, color))
        })
        .collect::<Vec<_>>();
    for (start, line) in source.split_inclusive('\n').scan(0, |offset, line| {
        let start = *offset;
        *offset += line.len();
        Some((start, line))
    }) {
        for (offset, _) in line.match_indices("//") {
            let at = start + offset;
            if !spans.iter().any(|(from, to, _)| *from <= at && at < *to) {
                spans.push((
                    at,
                    start + line.trim_end_matches('\n').len(),
                    theme.syntax.comment.0,
                ));
                break;
            }
        }
    }
    spans.sort_by_key(|(start, end, _)| (*start, *end));
    let mut job = LayoutJob::default();
    let mut cursor = 0;
    let mut append = |text: &str, color| {
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        )
    };
    for (start, end, color) in spans {
        if start < cursor || start >= end || end > source.len() {
            continue;
        }
        append(&source[cursor..start], theme.syntax.plain.0);
        append(&source[start..end], color);
        cursor = end;
    }
    append(&source[cursor..], theme.syntax.plain.0);
    job
}

fn editor_gutter_width(total_lines: usize, font_size: f32, mode: LineNumberMode) -> f32 {
    if mode == LineNumberMode::Off {
        20.0
    } else {
        34.0 + total_lines.max(1).to_string().len() as f32 * font_size * 0.48
    }
}

fn displayed_line_number(mode: LineNumberMode, row: usize, current_line: usize) -> Option<usize> {
    match mode {
        LineNumberMode::Off => None,
        LineNumberMode::Absolute => Some(row + 1),
        LineNumberMode::Relative => Some(row.abs_diff(current_line)),
        LineNumberMode::Hybrid if row == current_line => Some(row + 1),
        LineNumberMode::Hybrid => Some(row.abs_diff(current_line)),
    }
}

fn paint_line_numbers(
    ui: &egui::Ui,
    output: &egui::text_edit::TextEditOutput,
    current_line: usize,
    mode: LineNumberMode,
    theme: &Theme,
    font_size: f32,
    gutter: f32,
) {
    let left = output.galley_pos.x - gutter;
    let clip = egui::Rect::from_min_max(
        egui::pos2(left, output.response.rect.top()),
        output.response.rect.right_bottom(),
    )
    .intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(clip);
    let separator = output.galley_pos.x - 10.0;
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(left, clip.top()),
            egui::pos2(separator, clip.bottom()),
        ),
        0.0,
        theme.ui.editor.0,
    );
    let mut line = 0;
    let mut first_row = true;
    for row in &output.galley.rows {
        if first_row {
            let value = displayed_line_number(mode, line, current_line);
            if let Some(value) = value {
                painter.text(
                    egui::pos2(separator - 10.0, output.galley_pos.y + row.rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    value.to_string(),
                    FontId::monospace(font_size * 0.78),
                    if line == current_line {
                        theme.ui.accent.0
                    } else {
                        theme.ui.accent.0.gamma_multiply(0.72)
                    },
                );
            }
        }
        first_row = row.ends_with_newline;
        if row.ends_with_newline {
            line += 1;
        }
    }
}

fn css_color(color: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

fn apply_preview_theme(theme: &Theme) {
    set_preview_theme(
        &css_color(theme.ui.panel.0),
        &css_color(theme.ui.editor.0),
        &css_color(theme.ui.text.0),
        &css_color(theme.ui.muted.0),
        &css_color(theme.ui.accent.0),
        &css_color(theme.ui.selection.0),
    );
}

fn expand_greek_shortcuts(source: &mut String) {
    if !source.contains('\\') {
        return;
    }
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    let mut changed = false;
    while let Some(offset) = source[cursor..].find('\\') {
        let slash = cursor + offset;
        output.push_str(&source[cursor..slash]);
        let rest = &source[slash..];
        let replacement = GREEK
            .iter()
            .filter(|(shortcut, _)| rest.starts_with(shortcut))
            .filter(|(shortcut, _)| {
                !rest[shortcut.len()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphabetic)
            })
            .max_by_key(|(shortcut, _)| shortcut.len());
        if let Some((shortcut, symbol)) = replacement {
            output.push_str(symbol);
            cursor = slash + shortcut.len();
            changed = true;
        } else {
            output.push('\\');
            cursor = slash + 1;
        }
    }
    if changed {
        output.push_str(&source[cursor..]);
        *source = output;
    }
}

fn paint_web_logo(ui: &mut egui::Ui, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(27.0, 27.0), egui::Sense::hover());
    let center = rect.center();
    let stroke = egui::Stroke::new(2.4, theme.ui.accent.0);
    for (a, b) in [
        (egui::vec2(-8.0, -7.0), egui::vec2(7.0, -7.0)),
        (egui::vec2(-8.0, -7.0), egui::vec2(-1.0, 0.0)),
        (egui::vec2(-1.0, 0.0), egui::vec2(-8.0, 7.0)),
        (egui::vec2(-8.0, 7.0), egui::vec2(7.0, 7.0)),
    ] {
        ui.painter().line_segment([center + a, center + b], stroke);
    }
    ui.painter()
        .circle_filled(center + egui::vec2(8.0, -7.0), 1.8, stroke.color);
    ui.painter()
        .circle_filled(center + egui::vec2(8.0, 7.0), 1.8, stroke.color);
}

fn web_icon_button(ui: &mut egui::Ui, glyph: char, tooltip: &str, theme: &Theme) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(27.0, 27.0), egui::Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 2.0, theme.ui.muted.0.gamma_multiply(0.20));
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            2.0,
            egui::Stroke::new(1.0, theme.ui.accent.0),
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::new(16.0, FontFamily::Name("VS Code Codicons".into())),
        theme.ui.text.0,
    );
    response.on_hover_text(tooltip)
}

fn web_activity_button(
    ui: &mut egui::Ui,
    glyph: char,
    selected: bool,
    tooltip: &str,
    theme: &Theme,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(40.0, 38.0), egui::Sense::click());
    let fill = if selected {
        theme.title_bar_color()
    } else if response.hovered() {
        theme.ui.panel.0
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 4.0, fill);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            2.0,
            egui::Stroke::new(1.0, theme.ui.accent.0),
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::new(23.0, FontFamily::Name("VS Code Codicons".into())),
        if selected || response.hovered() {
            theme.ui.text.0
        } else {
            theme.ui.muted.0
        },
    );
    response.on_hover_text(tooltip)
}

fn web_tree_disclosure(ui: &mut egui::Ui, collapsed: bool, theme: &Theme) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::click());
    let glyph = if collapsed { '\u{eab6}' } else { '\u{eab4}' };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::new(14.0, FontFamily::Name("VS Code Codicons".into())),
        if response.hovered() {
            theme.ui.text.0
        } else {
            theme.ui.muted.0
        },
    );
    response
}

fn web_status_badge(ui: &mut egui::Ui, glyph: char, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::click());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::new(14.0, FontFamily::Name("VS Code Codicons".into())),
        color,
    );
    response.on_hover_text("Show Problems")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineNumberMode {
    Off,
    Absolute,
    Relative,
    Hybrid,
}

impl LineNumberMode {
    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Absolute => "Absolute",
            Self::Relative => "Relative",
            Self::Hybrid => "Relative + current",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Absolute => "absolute",
            Self::Relative => "relative",
            Self::Hybrid => "hybrid",
        }
    }

    fn from_key(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "absolute" => Some(Self::Absolute),
            "relative" => Some(Self::Relative),
            "hybrid" => Some(Self::Hybrid),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Document,
    Repl,
    Plot,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    Output,
    Problems,
}

struct WebSeries {
    label: String,
    samples: Vec<(f64, Option<f64>)>,
}

struct WebProblem {
    line: usize,
    column: usize,
    message: String,
}

struct WebReplEntry {
    input: String,
    output: String,
    error: Option<String>,
}

struct WebApp {
    context: egui::Context,
    source: String,
    output: String,
    error: Option<String>,
    problems: Vec<WebProblem>,
    document: Option<Document>,
    tex: String,
    tab: Tab,
    plot_expression: String,
    plot_series: Vec<WebSeries>,
    plot_bounds: [f64; 4],
    plot_svg: String,
    plot_custom_view: bool,
    plot_fit_y_pending: bool,
    plot_due: Option<f64>,
    repl_input: String,
    repl_entries: Vec<WebReplEntry>,
    repl_history_index: Option<usize>,
    last_repl_submission: String,
    queued_inputs: Vec<String>,
    generation: u64,
    pending: bool,
    due: Option<f64>,
    worker: Option<Worker>,
    _onmessage: Option<Closure<dyn FnMut(MessageEvent)>>,
    responses: Receiver<String>,
    status: String,
    source_name: String,
    preview_json: String,
    explorer_open: bool,
    preview_open: bool,
    preview_width: Option<f32>,
    output_open: bool,
    bottom_tab: BottomTab,
    highlight_source: String,
    highlight_font_size_bits: u32,
    highlight_job: LayoutJob,
    editor_font_size: f32,
    themes: Vec<Theme>,
    selected_theme: usize,
    theme_path: String,
    font_path: String,
    line_numbers: LineNumberMode,
    auto_greek: bool,
    dirty: bool,
    pending_new: bool,
    settings_open: bool,
    save_as_open: bool,
    save_as_name: String,
    cursor_line: usize,
    cursor_column: usize,
    cursor_char_index: usize,
    rename_target: Option<String>,
    rename_input: String,
    first_frame: bool,
    quick_open: bool,
    quick_open_query: String,
    explorer_root_open: bool,
    explorer_expanded: HashSet<String>,
}

impl WebApp {
    fn new(context: egui::Context) -> Self {
        let (sender, responses) = mpsc::channel();
        let (worker, onmessage, status) = match create_worker() {
            Ok(worker) => {
                let onmessage = Closure::wrap(Box::new(move |event: MessageEvent| {
                    if let Some(result) = event.data().as_string() {
                        let _ = sender.send(result);
                    }
                }) as Box<dyn FnMut(_)>);
                worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
                (Some(worker), Some(onmessage), "Ready".to_owned())
            }
            Err(_) => (
                None,
                None,
                "Web Worker unavailable; execution disabled".to_owned(),
            ),
        };
        let stored = web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .and_then(|storage| storage.get_item("scas.draft").ok().flatten());
        let editor_font_size = web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .and_then(|storage| storage.get_item("scas.editorFontSize").ok().flatten())
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(DEFAULT_EDITOR_FONT_SIZE)
            .clamp(MIN_EDITOR_FONT_SIZE, MAX_EDITOR_FONT_SIZE);
        let storage = web_sys::window().and_then(|window| window.local_storage().ok().flatten());
        let mut themes = Theme::bundled_themes();
        if let Some(custom) = storage
            .as_ref()
            .and_then(|storage| storage.get_item("scas.customThemeSource").ok().flatten())
            .and_then(|source| toml::from_str::<Theme>(&source).ok())
        {
            themes.push(custom);
        }
        let selected_theme = storage
            .as_ref()
            .and_then(|storage| storage.get_item("scas.theme").ok().flatten())
            .and_then(|name| themes.iter().position(|theme| theme.name == name))
            .unwrap_or(0);
        let line_numbers = storage
            .as_ref()
            .and_then(|storage| storage.get_item("scas.lineNumbers").ok().flatten())
            .and_then(|value| LineNumberMode::from_key(&value))
            .unwrap_or(LineNumberMode::Absolute);
        let auto_greek = storage
            .as_ref()
            .and_then(|storage| storage.get_item("scas.autoGreek").ok().flatten())
            .map_or(true, |value| value != "false");
        themes[selected_theme].apply(&context);
        apply_preview_theme(&themes[selected_theme]);
        let shared = web_sys::window()
            .and_then(|window| window.location().hash().ok())
            .and_then(|hash| hash.strip_prefix("#source=").map(str::to_owned))
            .and_then(|encoded| js_sys::decode_uri_component(&encoded).ok())
            .map(String::from);
        let dirty = stored.is_some();
        let source_name = if shared.is_some() {
            "shared.scas"
        } else if stored.is_some() {
            "untitled.scas"
        } else {
            "Untitled"
        };
        let source = shared.or(stored).unwrap_or_else(|| STARTER.to_owned());
        let initial_preview = if source_name == "Untitled" {
            document::build_live_with_limits(&source, crate::web_runtime::browser_limits()).ok()
        } else {
            None
        };
        let initial_tex = initial_preview
            .as_ref()
            .map_or_else(String::new, Document::latex);
        let initial_preview_json = initial_preview.as_ref().map_or_else(|| "[]".to_owned(), |document| {
            serde_json::to_string(&document.fragments.iter().map(|fragment| json!({
                "kind": match fragment.kind { FragmentKind::Prose => "prose", FragmentKind::DisplayMath => "display", FragmentKind::AlignedMath => "aligned" },
                "latex": fragment.latex,
                "span": { "line": fragment.span.line }
            })).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".to_owned())
        });
        let mut app = Self {
            context,
            source,
            output: String::new(),
            error: None,
            problems: Vec::new(),
            document: initial_preview,
            tex: initial_tex,
            tab: Tab::Document,
            plot_expression: "sin(x)".to_owned(),
            plot_series: Vec::new(),
            plot_bounds: [-10.0, 10.0, -1.0, 1.0],
            plot_svg: String::new(),
            plot_custom_view: false,
            plot_fit_y_pending: false,
            plot_due: None,
            repl_input: String::new(),
            repl_entries: Vec::new(),
            repl_history_index: None,
            last_repl_submission: String::new(),
            queued_inputs: Vec::new(),
            generation: 0,
            pending: false,
            due: None,
            worker,
            _onmessage: onmessage,
            responses,
            status,
            source_name: source_name.to_owned(),
            preview_json: initial_preview_json,
            explorer_open: true,
            preview_open: true,
            preview_width: None,
            output_open: false,
            bottom_tab: BottomTab::Output,
            highlight_source: String::new(),
            highlight_font_size_bits: 0,
            highlight_job: LayoutJob::default(),
            editor_font_size,
            themes,
            selected_theme,
            theme_path: String::new(),
            font_path: String::new(),
            line_numbers,
            auto_greek,
            dirty,
            pending_new: false,
            settings_open: false,
            save_as_open: false,
            save_as_name: "document.scas".to_owned(),
            cursor_line: 1,
            cursor_column: 1,
            cursor_char_index: 0,
            rename_target: None,
            rename_input: String::new(),
            first_frame: true,
            quick_open: false,
            quick_open_query: String::new(),
            explorer_root_open: true,
            explorer_expanded: HashSet::new(),
        };
        app.request("run");
        app
    }

    fn load_source(&mut self, source: String) {
        self.source = source;
        "imported.scas".clone_into(&mut self.source_name);
        self.dirty = false;
        self.save_draft();
        self.request("run");
    }

    fn new_file(&mut self) {
        if self.dirty {
            self.pending_new = true;
        } else {
            self.new_file_unchecked();
        }
    }

    fn new_file_unchecked(&mut self) {
        self.source = STARTER.to_owned();
        self.source_name = "Untitled".to_owned();
        self.tab = Tab::Document;
        self.dirty = false;
        self.pending_new = false;
        self.highlight_source.clear();
        self.save_draft();
        self.request("run");
    }

    fn import_data_file(&mut self, name: String, content: String) {
        let name = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_owned();
        if let Some(worker) = &self.worker {
            let payload =
                json!({"kind": "file_import", "generation": 0, "name": name, "content": content});
            if worker
                .post_message(&JsValue::from_str(&payload.to_string()))
                .is_ok()
            {
                self.status = format!("Imported {name}");
                self.context.request_repaint();
            }
        }
    }

    fn save_draft(&self) {
        if let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        {
            let _ = storage.set_item("scas.draft", &self.source);
        }
    }

    fn save_editor_preferences(&self) {
        if let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        {
            let _ = storage.set_item("scas.theme", &self.themes[self.selected_theme].name);
            let _ = storage.set_item("scas.lineNumbers", self.line_numbers.key());
            let _ = storage.set_item(
                "scas.autoGreek",
                if self.auto_greek { "true" } else { "false" },
            );
            let _ = storage.set_item("scas.editorFontSize", &self.editor_font_size.to_string());
        }
    }

    fn import_theme(&mut self, name: String, source: String) {
        match toml::from_str::<Theme>(&source) {
            Ok(theme) => {
                self.theme_path = name;
                self.themes.push(theme);
                self.selected_theme = self.themes.len() - 1;
                self.themes[self.selected_theme].apply(&self.context);
                apply_preview_theme(&self.themes[self.selected_theme]);
                self.highlight_source.clear();
                self.status = format!("Using {}", self.themes[self.selected_theme].name);
                if let Some(storage) =
                    web_sys::window().and_then(|window| window.local_storage().ok().flatten())
                {
                    let _ = storage.set_item("scas.customThemeSource", &source);
                }
                self.save_editor_preferences();
                self.context.request_repaint();
            }
            Err(error) => self.status = format!("Invalid theme: {error}"),
        }
    }

    fn install_font(&mut self, name: String, bytes: Vec<u8>) {
        if bytes.is_empty() {
            self.status = "Font file is empty".to_owned();
            return;
        }
        self.context
            .set_fonts(web_fonts(Some((name.clone(), bytes))));
        self.font_path = name.clone();
        self.highlight_source.clear();
        self.status = format!("Loaded font {name}");
        self.context.request_repaint();
    }

    fn request(&mut self, kind: &str) {
        self.generation = self.generation.wrapping_add(1);
        let request = json!({
            "generation": self.generation,
            "kind": kind,
            "source": self.source,
            "expression": self.plot_expression,
            "input": self.repl_input,
            "inputs": self.queued_inputs,
            "view": if kind == "plot" && self.plot_custom_view { Some(self.plot_bounds) } else { None },
            "fit_y": kind == "plot" && self.plot_fit_y_pending,
        });
        if let Some(worker) = &self.worker {
            if worker
                .post_message(&JsValue::from_str(&request.to_string()))
                .is_ok()
            {
                self.pending = true;
                (if kind == "plot" {
                    "Sampling plot…"
                } else {
                    "Evaluating…"
                })
                .clone_into(&mut self.status);
            } else {
                "Could not send work to the browser worker".clone_into(&mut self.status);
            }
        }
    }

    fn poll(&mut self, context: &egui::Context) {
        while let Ok(response) = self.responses.try_recv() {
            let Ok(value) = serde_json::from_str::<Value>(&response) else {
                continue;
            };
            if value["generation"].as_u64() != Some(self.generation) {
                continue;
            }
            self.pending = false;
            if let Some(prompt) = value["input_required"].as_str() {
                if let Some(answer) = prompt_input(prompt) {
                    self.queued_inputs.push(answer);
                    if value["kind"].as_str() == Some("repl") {
                        self.repl_input = self.last_repl_submission.clone();
                        self.request("repl");
                        self.repl_input.clear();
                    } else {
                        self.request("run");
                    }
                    continue;
                }
            } else {
                self.queued_inputs.clear();
            }
            if value["kind"].as_str() == Some("repl") {
                let entry = WebReplEntry {
                    input: value["input"].as_str().unwrap_or("").to_owned(),
                    output: value["output"].as_str().unwrap_or("").to_owned(),
                    error: value["error"].as_str().map(str::to_owned),
                };
                if entry.input.trim() == ":reset" && entry.error.is_none() {
                    self.repl_entries.clear();
                }
                self.status = if entry.error.is_some() {
                    "Needs attention"
                } else {
                    "Ready"
                }
                .to_owned();
                self.repl_entries.push(entry);
                context.request_repaint();
                continue;
            }
            self.error = value["error"].as_str().map(str::to_owned);
            self.problems = value["diagnostics"]
                .as_array()
                .map(|diagnostics| {
                    diagnostics
                        .iter()
                        .map(|diagnostic| WebProblem {
                            line: diagnostic["line"].as_u64().unwrap_or(1) as usize,
                            column: diagnostic["column"].as_u64().unwrap_or(1) as usize,
                            message: diagnostic["message"].as_str().unwrap_or("").to_owned(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if let Some(output) = value["output"].as_str() {
                output.clone_into(&mut self.output);
            }
            if let Some(tex) = value["tex"].as_str() {
                tex.clone_into(&mut self.tex);
            }
            if let Some(fragments) = value["fragments"].as_array() {
                let mut document = Document::default();
                for fragment in fragments {
                    let kind = match fragment["kind"].as_str() {
                        Some("prose") => FragmentKind::Prose,
                        Some("aligned") => FragmentKind::AlignedMath,
                        _ => FragmentKind::DisplayMath,
                    };
                    let span = &fragment["span"];
                    document.fragments.push(DocumentFragment {
                        kind,
                        latex: fragment["latex"].as_str().unwrap_or("").to_owned(),
                        span: SourceSpan {
                            line: span["line"].as_u64().unwrap_or(1) as usize,
                            column: span["column"].as_u64().unwrap_or(1) as usize,
                            length: span["length"].as_u64().unwrap_or(1) as usize,
                            byte_start: span["byte_start"].as_u64().unwrap_or(0) as usize,
                            byte_end: span["byte_end"].as_u64().unwrap_or(0) as usize,
                        },
                    });
                }
                if self.error.is_none() || self.document.is_none() {
                    self.preview_json =
                        serde_json::to_string(fragments).unwrap_or_else(|_| "[]".to_owned());
                    self.document = Some(document);
                }
            }
            if let Some(bounds) = value["bounds"].as_array() {
                if bounds.len() == 4 {
                    for (index, bound) in bounds.iter().enumerate() {
                        if !self.plot_custom_view || (self.plot_fit_y_pending && index >= 2) {
                            self.plot_bounds[index] =
                                bound.as_f64().unwrap_or(self.plot_bounds[index]);
                        }
                    }
                    self.plot_fit_y_pending = false;
                }
            }
            if let Some(series) = value["series"].as_array() {
                self.plot_series = series
                    .iter()
                    .map(|series| WebSeries {
                        label: series["label"].as_str().unwrap_or("curve").to_owned(),
                        samples: series["samples"]
                            .as_array()
                            .map(|samples| {
                                samples
                                    .iter()
                                    .filter_map(|point| {
                                        let pair = point.as_array()?;
                                        Some((pair.first()?.as_f64()?, pair.get(1)?.as_f64()))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .collect();
            }
            if let Some(svg) = value["svg"].as_str() {
                svg.clone_into(&mut self.plot_svg);
            }
            (if self.error.is_some() {
                "Needs attention"
            } else {
                "Ready"
            })
            .clone_into(&mut self.status);
            context.request_repaint();
        }
        if self.pending {
            context.request_repaint_after(Duration::from_millis(80));
        }
    }

    fn plot_view(&self) -> PlotBounds {
        let [x_min, x_max, y_min, y_max] = self.plot_bounds;
        PlotBounds::new(x_min, x_max, y_min, y_max)
    }

    fn apply_plot_view(&mut self, view: PlotBounds, now: f64, context: &egui::Context) {
        self.plot_bounds = [view.x_min, view.x_max, view.y_min, view.y_max];
        self.plot_custom_view = true;
        self.plot_fit_y_pending = false;
        self.plot_svg.clear();
        self.plot_due = Some(now + 0.14);
        context.request_repaint_after(Duration::from_millis(150));
    }

    fn zoom_plot(
        &mut self,
        factor: f64,
        x_ratio: f64,
        y_ratio: f64,
        now: f64,
        context: &egui::Context,
    ) {
        if let Some(view) = self.plot_view().zoomed_at(factor, x_ratio, y_ratio) {
            self.apply_plot_view(view, now, context);
        }
    }

    fn plot(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let now = ui.input(|input| input.time);
        let drag = if response.dragged() {
            ui.input(|input| input.pointer.delta())
        } else {
            egui::Vec2::ZERO
        };
        if drag != egui::Vec2::ZERO {
            if let Some(view) =
                self.plot_view()
                    .panned_by_pixels(drag.x, drag.y, rect.width(), rect.height())
            {
                self.apply_plot_view(view, now, ui.ctx());
            }
        }
        let (scroll, pointer) = ui.input(|input| {
            (
                input.raw_scroll_delta.y,
                input.pointer.hover_pos().unwrap_or(rect.center()),
            )
        });
        if response.hovered() && scroll.abs() > 0.0 {
            let factor = f64::from((-scroll * 0.0015).exp()).clamp(0.5, 2.0);
            let x_ratio = f64::from((pointer.x - rect.left()) / rect.width().max(1.0));
            let y_ratio = f64::from((pointer.y - rect.top()) / rect.height().max(1.0));
            self.zoom_plot(factor, x_ratio, y_ratio, now, ui.ctx());
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme.ui.background.0);
        let [x_min, x_max, y_min, y_max] = self.plot_bounds;
        let map = |x: f64, y: f64| {
            egui::pos2(
                rect.left() + ((x - x_min) / (x_max - x_min)) as f32 * rect.width(),
                rect.bottom() - ((y - y_min) / (y_max - y_min)) as f32 * rect.height(),
            )
        };
        let x_step = plot::nice_step(x_max - x_min, 10.0);
        let y_step = plot::nice_step(y_max - y_min, 8.0);
        for (vertical, minimum, maximum, step) in
            [(true, x_min, x_max, x_step), (false, y_min, y_max, y_step)]
        {
            if !step.is_finite() || step <= 0.0 {
                continue;
            }
            let mut value = (minimum / step).ceil() * step;
            for _ in 0..80 {
                if value > maximum {
                    break;
                }
                let zero = value.abs() < step * 1e-6;
                let color = if zero {
                    theme.ui.muted.0.gamma_multiply(0.55)
                } else {
                    theme.ui.muted.0.gamma_multiply(0.20)
                };
                let stroke = egui::Stroke::new(if zero { 1.4 } else { 1.0 }, color);
                if vertical {
                    let x = map(value, 0.0).x;
                    painter.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        stroke,
                    );
                    painter.text(
                        egui::pos2(x + 3.0, rect.bottom() - 4.0),
                        egui::Align2::LEFT_BOTTOM,
                        format_plot_tick(value, step),
                        FontId::monospace(10.0),
                        theme.ui.muted.0,
                    );
                } else {
                    let y = map(0.0, value).y;
                    painter.line_segment(
                        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                        stroke,
                    );
                    painter.text(
                        egui::pos2(rect.left() + 5.0, y - 3.0),
                        egui::Align2::LEFT_BOTTOM,
                        format_plot_tick(value, step),
                        FontId::monospace(10.0),
                        theme.ui.muted.0,
                    );
                }
                value += step;
            }
        }
        let colors = [
            Color32::from_rgb(79, 156, 249),
            Color32::from_rgb(249, 119, 127),
            Color32::from_rgb(83, 196, 137),
        ];
        for (index, series) in self.plot_series.iter().enumerate() {
            let mut previous = None;
            for &(x, y) in &series.samples {
                let point = y.filter(|y| y.is_finite()).map(|y| map(x, y));
                if let (Some(a), Some(b)) = (previous, point) {
                    painter
                        .line_segment([a, b], egui::Stroke::new(2.0, colors[index % colors.len()]));
                }
                previous = point;
            }
        }
        if self.plot_series.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Enter an expression above, then select Plot.",
                FontId::proportional(13.0),
                theme.ui.muted.0,
            );
        }
        painter.text(
            egui::pos2(rect.right() - 10.0, rect.bottom() - 10.0),
            egui::Align2::RIGHT_BOTTOM,
            "Drag to pan · Scroll to zoom",
            FontId::proportional(11.0),
            theme.ui.muted.0,
        );
    }
}

fn format_plot_tick(value: f64, step: f64) -> String {
    let digits = if step < 0.001 {
        6
    } else if step < 1.0 {
        3
    } else if step < 100.0 {
        1
    } else {
        0
    };
    let text = format!("{value:.digits$}");
    if digits == 0 {
        text
    } else {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

impl WebApp {
    fn submit_repl(&mut self) {
        if self.pending || self.repl_input.trim().is_empty() {
            return;
        }
        self.repl_input = self.repl_input.trim().to_owned();
        self.last_repl_submission = self.repl_input.clone();
        self.request("repl");
        self.repl_input.clear();
        self.repl_history_index = None;
    }

    fn show_repl(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        egui::Frame::none().fill(theme.ui.panel.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(30.0);
                ui.label(RichText::new("REPL").strong().size(12.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Reset session").clicked() && !self.pending {
                        self.repl_input = ":reset".to_owned();
                        self.submit_repl();
                    }
                });
            });
        });
        let muted = theme.ui.muted.0;
        let history_height = (ui.available_height() - 104.0).max(60.0);
        egui::ScrollArea::vertical()
            .id_source("web-repl-history")
            .max_height(history_height)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if self.repl_entries.is_empty() {
                    ui.add_space(26.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Start a calculation").strong().size(18.0));
                        ui.label(RichText::new("Enter an expression below. Definitions stay available for this browser session.").size(12.0).color(muted));
                        ui.monospace("Try: f(x) = x^2 - 2;   then   f(3)");
                    });
                }
                for (index, entry) in self.repl_entries.iter().enumerate() {
                    egui::Frame::none().inner_margin(egui::Margin::symmetric(12.0, 9.0)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}", index + 1)).monospace().size(10.5).color(muted));
                            ui.label(RichText::new(&entry.input).monospace().size(14.0));
                        });
                        if !entry.output.is_empty() {
                            ui.label(RichText::new(&entry.output).monospace().size(13.0));
                        }
                        if let Some(error) = &entry.error {
                            ui.colored_label(Color32::from_rgb(249, 119, 127), error);
                        }
                    });
                    ui.separator();
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(">")
                    .monospace()
                    .size(17.0)
                    .color(theme.ui.accent.0),
            );
            let response = ui.add_sized(
                [ui.available_width() - 75.0, 62.0],
                TextEdit::multiline(&mut self.repl_input)
                    .id(egui::Id::new("web-repl-input"))
                    .font(FontId::monospace(14.0))
                    .desired_rows(2)
                    .hint_text(
                        "SCAS expression or statement · Enter to run, Shift+Enter for a new line",
                    ),
            );
            if response.changed() && self.auto_greek {
                expand_greek_shortcuts(&mut self.repl_input);
            }
            if response.has_focus() {
                let (up, down, enter, shift) = ui.input(|input| {
                    (
                        input.key_pressed(egui::Key::ArrowUp),
                        input.key_pressed(egui::Key::ArrowDown),
                        input.key_pressed(egui::Key::Enter),
                        input.modifiers.shift,
                    )
                });
                if up && self.repl_input.trim().is_empty() {
                    let index = self.repl_entries.len().saturating_sub(1);
                    if let Some(entry) = self.repl_entries.get(index) {
                        self.repl_input.clone_from(&entry.input);
                        self.repl_history_index = Some(index);
                    }
                } else if down {
                    if let Some(index) = self.repl_history_index {
                        if let Some(next) = self.repl_entries.get(index + 1) {
                            self.repl_input.clone_from(&next.input);
                            self.repl_history_index = Some(index + 1);
                        } else {
                            self.repl_input.clear();
                            self.repl_history_index = None;
                        }
                    }
                }
                if enter && !shift {
                    self.submit_repl();
                }
            }
            if ui
                .add_enabled(
                    !self.pending && !self.repl_input.trim().is_empty(),
                    egui::Button::new("Run"),
                )
                .clicked()
            {
                self.submit_repl();
            }
        });
    }

    fn preview_panel(&self, ui: &mut egui::Ui, context: &egui::Context) {
        let rect = ui.max_rect();
        show_preview(
            &self.preview_json,
            &self.tex,
            self.pending,
            rect.left(),
            rect.top(),
            rect.width(),
            rect.height(),
            context.pixels_per_point(),
            true,
        );
    }

    fn show_settings(&mut self, context: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        let mut font_changed = false;
        let mut preferences_changed = false;
        let mut chosen_theme = None;
        let mut load_theme = false;
        let mut load_font = false;
        egui::Window::new("Editor settings")
            .open(&mut open)
            .resizable(false)
            .show(context, |ui| {
                ui.label("Color scheme");
                egui::ComboBox::from_id_source("theme-picker")
                    .selected_text(&self.themes[self.selected_theme].name)
                    .show_ui(ui, |ui| {
                        for (index, theme) in self.themes.iter().enumerate() {
                            if ui
                                .selectable_value(&mut self.selected_theme, index, &theme.name)
                                .changed()
                            {
                                chosen_theme = Some(index);
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Theme file");
                    ui.add(
                        TextEdit::singleline(&mut self.theme_path)
                            .hint_text("Choose a .toml theme")
                            .desired_width(210.0),
                    );
                    load_theme = ui.button("Load").clicked();
                });
                ui.separator();
                ui.label("Line numbers");
                let previous_line_numbers = self.line_numbers;
                egui::ComboBox::from_id_source("line-number-mode")
                    .selected_text(self.line_numbers.label())
                    .show_ui(ui, |ui| {
                        for mode in [
                            LineNumberMode::Off,
                            LineNumberMode::Absolute,
                            LineNumberMode::Relative,
                            LineNumberMode::Hybrid,
                        ] {
                            ui.selectable_value(&mut self.line_numbers, mode, mode.label());
                        }
                    });
                preferences_changed |= self.line_numbers != previous_line_numbers;
                preferences_changed |= ui
                    .checkbox(&mut self.auto_greek, "Expand LaTeX-style Greek shortcuts")
                    .changed();
                ui.separator();
                ui.label("Editor font");
                ui.horizontal(|ui| {
                    ui.label("Monospace");
                    font_changed |= ui
                        .add(
                            egui::Slider::new(
                                &mut self.editor_font_size,
                                MIN_EDITOR_FONT_SIZE..=MAX_EDITOR_FONT_SIZE,
                            )
                            .suffix(" pt"),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Font file");
                    ui.add(
                        TextEdit::singleline(&mut self.font_path)
                            .hint_text("Choose a .ttf or .otf font")
                            .desired_width(210.0),
                    );
                    load_font = ui.button("Load").clicked();
                });
                ui.small("JetBrains Mono is the default. Font files may be .ttf or .otf.");
                ui.small("Changes are saved automatically for the next launch.");
            });
        self.settings_open = open;
        if let Some(index) = chosen_theme {
            self.themes[index].apply(context);
            apply_preview_theme(&self.themes[index]);
            self.highlight_source.clear();
            self.status = format!("Using {}", self.themes[index].name);
            preferences_changed = true;
        }
        if load_theme {
            import_theme_file();
        }
        if load_font {
            import_font_file();
        }
        if font_changed || preferences_changed {
            self.highlight_source.clear();
            self.save_editor_preferences();
        }
    }

    fn show_save_as(&mut self, context: &egui::Context) {
        if !self.save_as_open {
            return;
        }
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Save As")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.label("Save the current document as a .scas file");
                let response =
                    ui.add_sized([420.0, 25.0], TextEdit::singleline(&mut self.save_as_name));
                if !response.has_focus() {
                    response.request_focus();
                }
                ui.horizontal(|ui| {
                    save = ui.button("Save").clicked()
                        || (response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if cancel {
            self.save_as_open = false;
        }
        if save {
            let trimmed = self.save_as_name.trim();
            if trimmed.is_empty() {
                self.status = "Enter a file name".to_owned();
            } else {
                let name = if trimmed.ends_with(".scas") {
                    trimmed.to_owned()
                } else {
                    format!("{trimmed}.scas")
                };
                download_file(&name, &self.source, "text/plain");
                self.source_name = name.clone();
                self.dirty = false;
                self.status = format!("Saved {name}");
                self.save_as_open = false;
                if self.pending_new {
                    self.new_file_unchecked();
                }
            }
        }
    }

    fn show_unsaved_changes(&mut self, context: &egui::Context) {
        if !self.pending_new || self.save_as_open {
            return;
        }
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.label("Save changes to this document?");
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        if self.source_name == "Untitled" {
                            self.save_as_name = "document.scas".to_owned();
                            self.save_as_open = true;
                        } else {
                            download_file(&self.source_name, &self.source, "text/plain");
                            self.dirty = false;
                            self.new_file_unchecked();
                        }
                    }
                    if ui.button("Discard").clicked() {
                        self.new_file_unchecked();
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_new = false;
                    }
                });
            });
    }

    fn begin_rename(&mut self) {
        let byte = char_to_byte(&self.source, self.cursor_char_index);
        if let Some(occurrence) = SourceIndex::new(&self.source).at(byte) {
            self.rename_input.clone_from(&occurrence.name);
            self.rename_target = Some(occurrence.name.clone());
        } else {
            self.status = "Place the cursor on a document symbol to rename it".to_owned();
        }
    }

    fn insert_editor_text(&mut self, template: &str) {
        let byte = char_to_byte(&self.source, self.cursor_char_index);
        let text = template.replace("$0", "");
        self.source.insert_str(byte, &text);
        self.cursor_char_index += text.chars().count();
        self.highlight_source.clear();
        self.dirty = true;
        self.save_draft();
        self.due = Some(self.context.input(|input| input.time) + 0.4);
        self.context
            .request_repaint_after(Duration::from_millis(420));
    }

    fn show_rename(&mut self, context: &egui::Context) {
        let Some(target) = self.rename_target.clone() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        egui::Window::new("Rename symbol")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(context, |ui| {
                ui.label(format!("Rename {target} across this document"));
                let response = ui.text_edit_singleline(&mut self.rename_input);
                response.request_focus();
                apply = ui.button("Rename").clicked()
                    || (response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                ui.label("Only unambiguous document-level symbols can be renamed.");
            });
        if !open {
            self.rename_target = None;
        }
        if apply {
            let index = SourceIndex::new(&self.source);
            if let Some(mut spans) = index.rename(&target, self.rename_input.trim()) {
                spans.sort_by_key(|span| span.start);
                for span in spans.iter().rev() {
                    self.source
                        .replace_range(span.clone(), self.rename_input.trim());
                }
                let count = spans.len();
                self.rename_target = None;
                self.highlight_source.clear();
                self.dirty = true;
                self.save_draft();
                self.request("run");
                self.status = format!(
                    "Renamed {target} to {} ({count} locations)",
                    self.rename_input.trim()
                );
            } else {
                self.status = "That symbol cannot be renamed unambiguously".to_owned();
            }
        }
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            if ui.button("New File").clicked() {
                self.new_file();
                ui.close_menu();
            }
            if ui.button("Quick Open…  Ctrl+P").clicked() {
                self.quick_open = true;
                ui.close_menu();
            }
            if ui.button("Save  Ctrl+S").clicked() {
                if self.source_name == "Untitled" {
                    self.save_as_name = "document.scas".to_owned();
                    self.save_as_open = true;
                } else {
                    download_file(&self.source_name, &self.source, "text/plain");
                    self.dirty = false;
                    self.status = format!("Saved {}", self.source_name);
                }
                ui.close_menu();
            }
            if ui.button("Save As…  Ctrl+Shift+S").clicked() {
                self.save_as_name = if self.source_name.ends_with(".scas") {
                    self.source_name.clone()
                } else {
                    "document.scas".to_owned()
                };
                self.save_as_open = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Export TeX…").clicked() {
                download_file("document.tex", &self.tex, "text/x-tex");
                ui.close_menu();
            }
            if ui.button("Export PDF…").clicked() {
                print_document();
                ui.close_menu();
            }
            if self.tab == Tab::Plot && ui.button("Export Graph as SVG…").clicked() {
                download_file("scas-plot.svg", &self.plot_svg, "image/svg+xml");
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Import files…").clicked() {
                import_file();
                ui.close_menu();
            }
        });
    }
}

impl eframe::App for WebApp {
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(context);
        if self.first_frame {
            if context.screen_rect().width() < 1100.0 {
                self.explorer_open = false;
            }
            if context.screen_rect().width() < 850.0 {
                self.preview_open = false;
            }
            self.first_frame = false;
        }
        let effective_editor_font_size = if context.screen_rect().width() < 600.0 {
            self.editor_font_size.min(18.0)
        } else {
            self.editor_font_size
        };
        let editor_font = FontId::monospace(effective_editor_font_size);
        let theme = self.themes[self.selected_theme].clone();
        if self.highlight_source != self.source
            || self.highlight_font_size_bits != effective_editor_font_size.to_bits()
        {
            self.highlight_job = highlight_source(&self.source, editor_font.clone(), &theme);
            self.highlight_source.clone_from(&self.source);
            self.highlight_font_size_bits = effective_editor_font_size.to_bits();
        }
        let now = context.input(|input| input.time);
        if self.due.is_some_and(|due| now >= due) {
            self.due = None;
            self.request("run");
        }
        if self.plot_due.is_some_and(|due| now >= due) {
            self.plot_due = None;
            self.request("plot");
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::B)) {
            self.explorer_open = !self.explorer_open;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::J)) {
            self.output_open = !self.output_open;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::P)) {
            self.quick_open = true;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F2)) {
            self.begin_rename();
        }
        if context.input_mut(|input| {
            input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::S,
            )
        }) {
            self.save_as_name = if self.source_name.ends_with(".scas") {
                self.source_name.clone()
            } else {
                "document.scas".to_owned()
            };
            self.save_as_open = true;
        } else if context
            .input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
        {
            if self.source_name == "Untitled" {
                self.save_as_open = true;
            } else {
                download_file(&self.source_name, &self.source, "text/plain");
                self.dirty = false;
                self.status = format!("Saved {}", self.source_name);
            }
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)) {
            if self.tab == Tab::Repl {
                self.submit_repl();
            } else if self.tab == Tab::Document {
                self.request("run");
                self.output_open = true;
            } else {
                self.request("plot");
            }
        }
        let dark = theme.ui.background.0;
        let panel = theme.ui.panel.0;
        let muted = theme.ui.muted.0;
        let border = egui::Stroke::new(1.0, muted.gamma_multiply(0.28));
        let mut jump_to = None;
        let mut preview_visible = false;
        let narrow = context.screen_rect().width() < 850.0;
        if narrow
            && self.preview_open
            && context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.preview_open = false;
        }

        egui::TopBottomPanel::top("web-title-bar")
            .exact_height(38.0)
            .frame(
                egui::Frame::none()
                    .fill(theme.title_bar_color())
                    .inner_margin(egui::Margin::symmetric(7.0, 4.0)),
            )
            .show(context, |ui| {
                let available = ui.available_width();
                let compact = available < 700.0;
                let left_width = if compact {
                    160.0
                } else {
                    (available * 0.32).clamp(410.0, 690.0)
                };
                let center_width = if compact {
                    (available - left_width - 90.0).max(80.0)
                } else {
                    (available * 0.38).clamp(350.0, 970.0)
                };
                ui.horizontal_centered(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_width, 30.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(left_width);
                            paint_web_logo(ui, &theme);
                            ui.add_space(9.0);
                            let button_frame = ui.visuals().button_frame;
                            ui.visuals_mut().button_frame = false;
                            self.file_menu(ui);
                            ui.menu_button("Edit", |ui| {
                                if ui
                                    .checkbox(&mut self.auto_greek, "Greek shortcuts")
                                    .clicked()
                                {
                                    self.save_editor_preferences();
                                    ui.close_menu();
                                }
                                if ui.button("Settings…").clicked() {
                                    self.settings_open = true;
                                    ui.close_menu();
                                }
                            });
                            ui.menu_button("View", |ui| {
                                ui.checkbox(&mut self.explorer_open, "Explorer  Ctrl+B");
                                ui.checkbox(&mut self.preview_open, "Document Preview");
                                ui.checkbox(&mut self.output_open, "Output Panel  Ctrl+J");
                                if ui.button("Problems").clicked() {
                                    self.bottom_tab = BottomTab::Problems;
                                    self.output_open = true;
                                    ui.close_menu();
                                }
                                if ui.button("Terminal").clicked() {
                                    self.tab = Tab::Repl;
                                    ui.close_menu();
                                }
                            });
                            if !compact {
                                ui.menu_button("Go", |ui| {
                                    if ui.button("Go to File…  Ctrl+P").clicked() {
                                        self.quick_open = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("Editor").clicked() {
                                        self.tab = Tab::Document;
                                        ui.close_menu();
                                    }
                                });
                            }
                            ui.menu_button("Run", |ui| {
                                if ui.button("Run Active  Ctrl+Enter").clicked() {
                                    match self.tab {
                                        Tab::Document => {
                                            self.request("run");
                                            self.output_open = true;
                                        }
                                        Tab::Repl => self.submit_repl(),
                                        Tab::Plot => self.request("plot"),
                                    }
                                    ui.close_menu();
                                }
                                if ui.button("Graphing Calculator").clicked() {
                                    self.tab = Tab::Plot;
                                    ui.close_menu();
                                }
                            });
                            if !compact {
                                ui.menu_button("Terminal", |ui| {
                                    if ui.button("New Terminal").clicked() {
                                        self.tab = Tab::Repl;
                                        ui.close_menu();
                                    }
                                    if ui.button("Interactive REPL").clicked() {
                                        self.tab = Tab::Repl;
                                        ui.close_menu();
                                    }
                                });
                            }
                            ui.visuals_mut().button_frame = button_frame;
                        },
                    );
                    ui.add_space(5.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(center_width, 30.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(center_width);
                            if ui
                                .add_sized(
                                    [center_width - 2.0, 26.0],
                                    egui::Button::new(RichText::new("scas").size(12.0))
                                        .fill(theme.ui.selection.0.gamma_multiply(0.45))
                                        .stroke(border),
                                )
                                .on_hover_text("Quick Open · Ctrl+P")
                                .clicked()
                            {
                                self.quick_open = true;
                            }
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if web_icon_button(ui, '\u{ebf4}', "Toggle preview", &theme).clicked() {
                            self.preview_open = !self.preview_open;
                        }
                        if web_icon_button(ui, '\u{ebf2}', "Toggle output panel", &theme).clicked()
                        {
                            self.output_open = !self.output_open;
                        }
                        if web_icon_button(ui, '\u{ebf3}', "Toggle Explorer", &theme).clicked() {
                            self.explorer_open = !self.explorer_open;
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("web-status")
            .exact_height(26.0)
            .frame(
                egui::Frame::none()
                    .fill(theme.ui.accent.0)
                    .inner_margin(egui::Margin::symmetric(8.0, 2.0)),
            )
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    let status_color = theme.contrast_on_accent();
                    let error_badge = web_status_badge(ui, '\u{ea87}', status_color);
                    ui.label(
                        RichText::new(self.problems.len().to_string())
                            .color(status_color)
                            .size(11.0),
                    );
                    let info_badge = web_status_badge(ui, '\u{ea74}', status_color);
                    ui.label(RichText::new("0").color(status_color).size(11.0));
                    if error_badge.clicked() || info_badge.clicked() {
                        self.bottom_tab = BottomTab::Problems;
                        self.output_open = true;
                    }
                    ui.label(
                        RichText::new(&self.status)
                            .color(theme.contrast_on_accent())
                            .size(11.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "Ln {}, Col {}    Spaces: 4    UTF-8    CRLF    {{ }} SCAS",
                                self.cursor_line, self.cursor_column
                            ))
                            .color(theme.contrast_on_accent())
                            .size(11.0),
                        );
                    });
                });
            });
        egui::SidePanel::left("web-activity")
            .exact_width(46.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(theme.activity_bar_color())
                    .stroke(border)
                    .inner_margin(egui::Margin::symmetric(3.0, 6.0)),
            )
            .show(context, |ui| {
                ui.vertical_centered(|ui| {
                    if web_activity_button(
                        ui,
                        '\u{eaf0}',
                        self.explorer_open,
                        "Explorer · Ctrl+B",
                        &theme,
                    )
                    .clicked()
                    {
                        self.explorer_open = !self.explorer_open;
                    }
                    if web_activity_button(
                        ui,
                        '\u{ea6d}',
                        self.quick_open,
                        "Quick Open · Ctrl+P",
                        &theme,
                    )
                    .clicked()
                    {
                        self.quick_open = true;
                    }
                    if web_activity_button(
                        ui,
                        '\u{ea85}',
                        self.tab == Tab::Repl,
                        "Interactive REPL",
                        &theme,
                    )
                    .clicked()
                    {
                        self.tab = Tab::Repl;
                    }
                    if web_activity_button(
                        ui,
                        '\u{ebe2}',
                        self.tab == Tab::Plot,
                        "Graphing",
                        &theme,
                    )
                    .clicked()
                    {
                        self.tab = Tab::Plot;
                    }
                });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if web_activity_button(
                        ui,
                        '\u{eb51}',
                        self.settings_open,
                        "Editor settings",
                        &theme,
                    )
                    .clicked()
                    {
                        self.settings_open = true;
                    }
                });
            });
        if self.explorer_open {
            egui::SidePanel::left("web-explorer")
                .default_width(300.0)
                .width_range(200.0..=420.0)
                .resizable(true)
                .frame(
                    egui::Frame::none()
                        .fill(panel)
                        .stroke(border)
                        .inner_margin(egui::Margin::symmetric(12.0, 3.0)),
                )
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Explorer").strong().size(13.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let button_frame = ui.visuals().button_frame;
                            ui.visuals_mut().button_frame = false;
                            ui.menu_button(
                                RichText::new('\u{ea7c}').font(FontId::new(
                                    16.0,
                                    FontFamily::Name("VS Code Codicons".into()),
                                )),
                                |ui| {
                                    if ui.button("New File").clicked() {
                                        self.new_file();
                                        ui.close_menu();
                                    }
                                    if ui.button("Refresh Files").clicked() {
                                        self.request("run");
                                        ui.close_menu();
                                    }
                                    if ui.button("Collapse Folders").clicked() {
                                        self.explorer_expanded.clear();
                                        ui.close_menu();
                                    }
                                },
                            );
                            ui.visuals_mut().button_frame = button_frame;
                        });
                    });
                    ui.horizontal(|ui| {
                        if web_tree_disclosure(ui, !self.explorer_root_open, &theme).clicked() {
                            self.explorer_root_open = !self.explorer_root_open;
                        }
                        if ui
                            .selectable_label(false, RichText::new("scas").strong())
                            .clicked()
                        {
                            self.explorer_root_open = !self.explorer_root_open;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if web_icon_button(ui, '\u{eac5}', "Collapse folders", &theme).clicked()
                            {
                                self.explorer_expanded.clear();
                            }
                            if web_icon_button(ui, '\u{eb37}', "Refresh workspace", &theme)
                                .clicked()
                            {
                                self.request("run");
                            }
                            if web_icon_button(ui, '\u{ea7f}', "New document", &theme).clicked() {
                                self.new_file();
                            }
                        });
                    });
                    if self.explorer_root_open {
                        ui.spacing_mut().item_spacing.y = 5.0;
                        for folder in [
                            "assets", "benches", "docs", "examples", "scripts", "src", "stdlib",
                            "tests", "themes", "web",
                        ] {
                            let expanded = self.explorer_expanded.contains(folder);
                            ui.horizontal(|ui| {
                                ui.add_space(7.0);
                                if web_tree_disclosure(ui, !expanded, &theme).clicked() {
                                    if expanded {
                                        self.explorer_expanded.remove(folder);
                                    } else {
                                        self.explorer_expanded.insert(folder.to_owned());
                                    }
                                }
                                ui.label(
                                    RichText::new('\u{ea83}')
                                        .font(FontId::new(
                                            15.0,
                                            FontFamily::Name("VS Code Codicons".into()),
                                        ))
                                        .color(muted),
                                );
                                if ui.selectable_label(false, folder).clicked() {
                                    if expanded {
                                        self.explorer_expanded.remove(folder);
                                    } else {
                                        self.explorer_expanded.insert(folder.to_owned());
                                    }
                                }
                            });
                            if expanded && folder == "examples" {
                                ui.indent("web-examples", |ui| {
                                    if ui.button("Starter.scas").clicked() {
                                        self.load_source(STARTER.to_owned());
                                        self.source_name = "Starter.scas".to_owned();
                                        self.tab = Tab::Document;
                                    }
                                    if ui.button("MIT 18.06.scas").clicked() {
                                        self.load_source(MIT_EXAMPLE.to_owned());
                                        self.source_name = "MIT 18.06.scas".to_owned();
                                        self.tab = Tab::Document;
                                    }
                                });
                            }
                        }
                    }
                    ui.add_space(7.0);
                    ui.separator();
                    ui.collapsing("Outline", |ui| {
                        ui.label("SCAS document symbols");
                    });
                    ui.collapsing("Timeline", |ui| {
                        ui.label("No timeline entries");
                    });
                });
        }
        if self.tab == Tab::Document {
            if self.output_open {
                egui::TopBottomPanel::bottom("web-output")
                    .resizable(true)
                    .default_height(230.0)
                    .height_range(120.0..=500.0)
                    .frame(
                        egui::Frame::none()
                            .fill(dark)
                            .stroke(border)
                            .inner_margin(egui::Margin::symmetric(10.0, 6.0)),
                    )
                    .show(context, |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(self.bottom_tab == BottomTab::Output, "OUTPUT")
                                .clicked()
                            {
                                self.bottom_tab = BottomTab::Output;
                            }
                            if ui
                                .selectable_label(
                                    self.bottom_tab == BottomTab::Problems,
                                    format!("PROBLEMS  {}", self.problems.len()),
                                )
                                .clicked()
                            {
                                self.bottom_tab = BottomTab::Problems;
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("×").on_hover_text("Close panel").clicked()
                                    {
                                        self.output_open = false;
                                    }
                                },
                            );
                        });
                        ui.separator();
                        egui::ScrollArea::vertical().show(ui, |ui| match self.bottom_tab {
                            BottomTab::Output => {
                                if !self.output.is_empty() {
                                    ui.monospace(&self.output);
                                } else {
                                    ui.label(RichText::new("No output yet.").color(muted));
                                }
                                if let Some(error) = &self.error {
                                    ui.colored_label(Color32::from_rgb(249, 119, 127), error);
                                }
                            }
                            BottomTab::Problems => {
                                if self.problems.is_empty() {
                                    ui.label(RichText::new("No problems detected.").color(muted));
                                }
                                for problem in &self.problems {
                                    let label = format!(
                                        "{}:{}:{}  {}",
                                        self.source_name,
                                        problem.line,
                                        problem.column,
                                        problem.message
                                    );
                                    if ui
                                        .selectable_label(
                                            false,
                                            RichText::new(label)
                                                .color(Color32::from_rgb(249, 119, 127)),
                                        )
                                        .clicked()
                                    {
                                        jump_to = Some((problem.line, problem.column));
                                    }
                                }
                            }
                        });
                    });
            }
            if self.preview_open {
                preview_visible = true;
                if !narrow {
                    let explorer_width = if self.explorer_open {
                        egui::containers::panel::PanelState::load(
                            context,
                            egui::Id::new("web-explorer"),
                        )
                        .map_or(300.0, |state| state.size().x)
                    } else {
                        0.0
                    };
                    let (preview_width, preview_max_width) =
                        preview_widths(context.screen_rect().width(), explorer_width);
                    let preview_width = self
                        .preview_width
                        .unwrap_or(preview_width)
                        .clamp(MIN_PREVIEW_WIDTH, preview_max_width);
                    egui::SidePanel::right("web-preview")
                        .resizable(false)
                        .exact_width(preview_width)
                        .frame(
                            egui::Frame::none()
                                .fill(panel)
                                .stroke(border)
                                .inner_margin(egui::Margin::symmetric(10.0, 7.0)),
                        )
                        .show(context, |ui| self.preview_panel(ui, context));
                }
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(dark))
                .show(context, |ui| {
                    egui::Frame::none().fill(panel).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            egui::Frame::none()
                                .fill(dark)
                                .inner_margin(egui::Margin::symmetric(10.0, 2.0))
                                .show(ui, |ui| {
                                    ui.horizontal_centered(|ui| {
                                        ui.set_min_height(30.0);
                                        ui.label(
                                            RichText::new("SC")
                                                .strong()
                                                .size(12.0)
                                                .color(theme.ui.accent.0),
                                        );
                                        ui.label(RichText::new(&self.source_name).size(12.0));
                                        if web_icon_button(
                                            ui,
                                            '\u{ea76}',
                                            "Reset to a new document",
                                            &theme,
                                        )
                                        .clicked()
                                        {
                                            self.new_file();
                                        }
                                    });
                                });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.menu_button(
                                        RichText::new('\u{ea7c}').font(FontId::new(
                                            16.0,
                                            FontFamily::Name("VS Code Codicons".into()),
                                        )),
                                        |ui| {
                                            if ui.button("Show Explorer").clicked() {
                                                self.explorer_open = true;
                                                ui.close_menu();
                                            }
                                            if ui.button("Show Preview").clicked() {
                                                self.preview_open = true;
                                                ui.close_menu();
                                            }
                                            if ui.button("Show Output").clicked() {
                                                self.output_open = true;
                                                ui.close_menu();
                                            }
                                        },
                                    );
                                    if web_icon_button(
                                        ui,
                                        '\u{ebf4}',
                                        "Toggle mathematical preview",
                                        &theme,
                                    )
                                    .clicked()
                                    {
                                        self.preview_open = !self.preview_open;
                                    }
                                    if web_icon_button(
                                        ui,
                                        '\u{eb2c}',
                                        "Run active workspace · Ctrl+Enter",
                                        &theme,
                                    )
                                    .clicked()
                                    {
                                        self.request("run");
                                        self.output_open = true;
                                    }
                                    ui.menu_button("Ω", |ui| {
                                        for (symbol, insertion) in
                                            [("π", "π"), ("θ", "θ"), ("λ", "λ"), ("∫", "integral(")]
                                        {
                                            if ui.button(symbol).clicked() {
                                                self.insert_editor_text(insertion);
                                                ui.close_menu();
                                            }
                                        }
                                    })
                                    .response
                                    .on_hover_text("Insert a mathematical symbol");
                                    ui.menu_button("{}", |ui| {
                                        for (label, detail, insertion) in [
                                            (
                                                "Document block",
                                                "multi-line documented calculation",
                                                "\\{\n    $0\n\\}\n",
                                            ),
                                            (
                                                "Display result",
                                                "render one dynamic result",
                                                "\\[ $0 => \\]",
                                            ),
                                            (
                                                "Function",
                                                "block function",
                                                "fn name(args) {\n    $0\n}",
                                            ),
                                            (
                                                "If / else",
                                                "conditional branches",
                                                "if condition {\n    $0\n} else {\n    \n}",
                                            ),
                                            (
                                                "For loop",
                                                "counted loop",
                                                "for let i = 0; i < n; i += 1 {\n    $0\n}",
                                            ),
                                            ("Matrix", "2 × 2 matrix", "[$0, 0; 0, 1]"),
                                            (
                                                "Document RREF",
                                                "show every row operation",
                                                "rref($0) =>;",
                                            ),
                                            (
                                                "Solve system",
                                                "step-by-step linear solve",
                                                "solve_linear_steps($0, b)",
                                            ),
                                        ] {
                                            if ui.button(label).on_hover_text(detail).clicked() {
                                                self.insert_editor_text(insertion);
                                                ui.close_menu();
                                            }
                                        }
                                    })
                                    .response
                                    .on_hover_text("Insert a code snippet");
                                },
                            );
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        ui.label(
                            RichText::new("SC")
                                .strong()
                                .size(10.0)
                                .color(theme.ui.accent.0),
                        );
                        ui.label(RichText::new(&self.source_name).size(12.0).color(muted));
                        ui.label(RichText::new("› …").size(12.0).color(muted));
                    });
                    ui.add_space(4.0);
                    let available_rows =
                        (ui.available_height() / (effective_editor_font_size + 1.0)) as usize;
                    let cached_source = &self.highlight_source;
                    let cached_job = &self.highlight_job;
                    let mut layouter = |ui: &egui::Ui, source: &str, width: f32| {
                        let mut job = if source == cached_source {
                            cached_job.clone()
                        } else {
                            highlight_source(source, editor_font.clone(), &theme)
                        };
                        job.wrap.max_width = width;
                        ui.fonts(|fonts| fonts.layout_job(job))
                    };
                    let gutter = editor_gutter_width(
                        self.source.lines().count(),
                        effective_editor_font_size,
                        self.line_numbers,
                    );
                    let (response, cursor_range) = egui::ScrollArea::both()
                        .show(ui, |ui| {
                            let output = TextEdit::multiline(&mut self.source)
                                .id(egui::Id::new("web-source"))
                                .font(editor_font.clone())
                                .code_editor()
                                .frame(false)
                                .margin(egui::Margin {
                                    left: gutter,
                                    right: 6.0,
                                    top: 4.0,
                                    bottom: 4.0,
                                })
                                .desired_width(ui.available_width())
                                .layouter(&mut layouter)
                                .desired_rows(available_rows.max(12))
                                .show(ui);
                            if self.line_numbers != LineNumberMode::Off {
                                paint_line_numbers(
                                    ui,
                                    &output,
                                    self.cursor_line.saturating_sub(1),
                                    self.line_numbers,
                                    &theme,
                                    effective_editor_font_size,
                                    gutter,
                                );
                            }
                            (output.response, output.cursor_range)
                        })
                        .inner;
                    if let Some(cursor) = cursor_range {
                        self.cursor_char_index = cursor.primary.ccursor.index;
                        (self.cursor_line, self.cursor_column) =
                            line_column_at_char(&self.source, self.cursor_char_index);
                    }
                    if response.changed() {
                        if self.auto_greek {
                            expand_greek_shortcuts(&mut self.source);
                        }
                        self.dirty = true;
                        self.save_draft();
                        self.due = Some(now + 0.4);
                        context.request_repaint_after(Duration::from_millis(420));
                    }
                });
            if let Some((line, column)) = jump_to {
                let index = self
                    .source
                    .lines()
                    .take(line.saturating_sub(1))
                    .map(|line| line.chars().count() + 1)
                    .sum::<usize>()
                    + column.saturating_sub(1);
                let editor_id = egui::Id::new("web-source");
                let mut state = TextEdit::load_state(context, editor_id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(
                        egui::text::CCursor::new(index.min(self.source.chars().count())),
                    )));
                TextEdit::store_state(context, editor_id, state);
                context.memory_mut(|memory| memory.request_focus(editor_id));
            }
        } else if self.tab == Tab::Repl {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(dark))
                .show(context, |ui| self.show_repl(ui, &theme));
        } else {
            egui::TopBottomPanel::top("plot-controls")
                .frame(egui::Frame::none().fill(panel))
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("y =");
                        let expression = ui.text_edit_singleline(&mut self.plot_expression);
                        if expression.changed() && self.auto_greek {
                            expand_greek_shortcuts(&mut self.plot_expression);
                        }
                        if ui.button("Plot").clicked()
                            || (expression.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                        {
                            self.plot_custom_view = false;
                            self.plot_fit_y_pending = false;
                            self.plot_due = None;
                            self.request("plot");
                        }
                        ui.separator();
                        if ui.button("+").on_hover_text("Zoom in").clicked() {
                            self.zoom_plot(0.75, 0.5, 0.5, now, context);
                        }
                        if ui.button("−").on_hover_text("Zoom out").clicked() {
                            self.zoom_plot(1.0 / 0.75, 0.5, 0.5, now, context);
                        }
                        if ui
                            .button("Fit y")
                            .on_hover_text("Fit the vertical range to the visible samples")
                            .clicked()
                        {
                            self.plot_custom_view = true;
                            self.plot_fit_y_pending = true;
                            self.request("plot");
                        }
                        if ui.button("Reset view").clicked() {
                            self.plot_custom_view = false;
                            self.plot_fit_y_pending = false;
                            self.request("plot");
                        }
                        if ui
                            .add_enabled(
                                !self.plot_svg.is_empty(),
                                egui::Button::new("Download SVG"),
                            )
                            .clicked()
                        {
                            download_file("scas-plot.svg", &self.plot_svg, "image/svg+xml");
                        }
                    });
                    if let Some(error) = &self.error {
                        ui.colored_label(Color32::from_rgb(249, 119, 127), error);
                    }
                    for series in &self.plot_series {
                        ui.label(RichText::new(&series.label).size(11.0));
                    }
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(dark))
                .show(context, |ui| self.plot(ui, &theme));
        }
        if narrow && self.tab == Tab::Document && self.preview_open {
            let screen = context.screen_rect();
            show_preview(
                &self.preview_json,
                &self.tex,
                self.pending,
                46.0,
                38.0,
                screen.width() - 46.0,
                screen.height() - 64.0,
                context.pixels_per_point(),
                false,
            );
        }
        if !preview_visible {
            hide_preview();
        }
        self.show_settings(context);
        self.show_save_as(context);
        self.show_unsaved_changes(context);
        self.show_rename(context);
        if self.quick_open {
            let mut open = true;
            let mut selected = None;
            egui::Window::new("Quick Open")
                .id(egui::Id::new("quick-open-window"))
                .title_bar(false)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 58.0))
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(context, |ui| {
                    ui.set_min_width(470.0_f32.min(context.screen_rect().width() - 48.0));
                    let response = ui.add(
                        TextEdit::singleline(&mut self.quick_open_query)
                            .id_source("quick-open-query")
                            .font(egui::TextStyle::Body)
                            .hint_text("Search SCAS files by name")
                            .desired_width(f32::INFINITY),
                    );
                    if !response.has_focus() {
                        response.request_focus();
                    }
                    if response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    {
                        selected = Some(0);
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(290.0)
                        .show(ui, |ui| {
                            let query = self.quick_open_query.to_lowercase();
                            for (index, label) in [
                                "examples/mit-18-06-session-22.scas",
                                "examples/starter.scas",
                                "Graphing calculator",
                                "Interactive REPL",
                            ]
                            .iter()
                            .enumerate()
                            {
                                if label.to_lowercase().contains(&query)
                                    && ui.selectable_label(false, *label).clicked()
                                {
                                    selected = Some(index);
                                }
                            }
                        });
                });
            if context.input(|input| input.key_pressed(egui::Key::Escape)) {
                open = false;
            }
            self.quick_open = open;
            if let Some(index) = selected {
                match index {
                    0 => {
                        self.load_source(MIT_EXAMPLE.to_owned());
                        "mit-18-06-session-22.scas".clone_into(&mut self.source_name);
                        self.tab = Tab::Document;
                    }
                    1 => {
                        self.load_source(STARTER.to_owned());
                        "starter.scas".clone_into(&mut self.source_name);
                        self.tab = Tab::Document;
                    }
                    2 => self.tab = Tab::Plot,
                    _ => self.tab = Tab::Repl,
                }
                self.quick_open = false;
                self.quick_open_query.clear();
            }
        }
    }
}
