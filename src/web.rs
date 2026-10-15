//! Browser entry point for a real egui SCAS workspace. The native IDE remains
//! separate while platform services are being extracted from its binary.

use crate::ast::statement::FunctionBody;
use crate::ast::{SourceSpan, StatementKind};
use crate::document::{self, Document, DocumentFragment, FragmentKind};
use crate::intelligence::SourceIndex;
use crate::latex::LatexRenderer;
use crate::lexer::{Scanner, TokenType};
use crate::parser::Parser;
use crate::plot::{self, CriticalPoint, PlotBounds};
use crate::web_runtime::{MIT_EXAMPLE, STARTER};
use eframe::egui::{
    self,
    text::{LayoutJob, TextFormat},
    Color32, FontData, FontDefinitions, FontFamily, FontId, RichText, TextEdit,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, Worker};

#[allow(dead_code)]
#[path = "bin/scas_gui/editor_assist.rs"]
mod editor_assist;
#[path = "bin/scas_gui/theme.rs"]
#[allow(dead_code)]
mod theme;
use theme::Theme;

const PLOT_COLORS: [Color32; 3] = [
    Color32::from_rgb(79, 156, 249),
    Color32::from_rgb(249, 119, 127),
    Color32::from_rgb(83, 196, 137),
];

fn graph_colors(theme: &Theme) -> [Color32; 3] {
    if theme.dark {
        PLOT_COLORS
    } else {
        [
            Color32::from_rgb(0, 105, 190),
            Color32::from_rgb(184, 35, 60),
            Color32::from_rgb(22, 116, 67),
        ]
    }
}

const DEFAULT_EDITOR_FONT_SIZE: f32 = 27.0;
const MIN_EDITOR_FONT_SIZE: f32 = 9.0;
const MAX_EDITOR_FONT_SIZE: f32 = 32.0;
const MIN_EDITOR_WORKSPACE_WIDTH: f32 = 420.0;
const MIN_PREVIEW_WIDTH: f32 = 220.0;
const MAX_PREVIEW_WIDTH: f32 = 900.0;
const ANALYSIS_TIMEOUT_SECONDS: f64 = 8.0;
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
    fn print_document(fragments: &str, tex: &str);
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
        error: &str,
        pending: bool,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        pixels_per_point: f32,
        resizable: bool,
    );
    #[wasm_bindgen(js_namespace = window, js_name = scasReplResult)]
    fn receive_repl_result(result: &str);
    #[wasm_bindgen(js_namespace = window, js_name = scasShowReplSurface)]
    fn show_repl_surface(
        surface: &str,
        content: &str,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        pixels_per_point: f32,
    );
    #[wasm_bindgen(js_namespace = window, js_name = scasHideRepl)]
    fn hide_repl();
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

    pub fn load_source_named(&self, name: String, source: String) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.load_source_named(name, source);
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

    pub fn jump_to_source(&self, line: usize, column: usize) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.jump_to_source(line, column);
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.runner.app_mut::<WebApp>().is_some_and(|app| {
            app.dirty
                || app
                    .documents
                    .iter()
                    .enumerate()
                    .any(|(index, document)| index != app.active_document && document.dirty)
        })
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

fn terminate_repl_source(source: &str) -> String {
    let trimmed = source.trim_end();
    if trimmed.ends_with(';') || trimmed.ends_with('}') {
        source.to_owned()
    } else {
        format!("{source};")
    }
}

fn repl_expression_latex(source: &str) -> Option<String> {
    let source = source.trim();
    if source.is_empty() || source.starts_with(':') {
        return None;
    }
    let submission = terminate_repl_source(source);
    let mut scanner = Scanner::new(&submission);
    let tokens = scanner.scan_tokens().clone();
    if scanner.had_error() {
        return None;
    }
    let symbols = scanner.into_symbols();
    let program = Parser::new(tokens).parse().ok()?;
    let renderer = LatexRenderer::new(&symbols);
    match &program.last()?.kind {
        StatementKind::Expr(expression) | StatementKind::Print(expression) => {
            Some(renderer.expression(expression))
        }
        StatementKind::Let {
            name,
            value: Some(value),
        } => Some(format!(
            "{} = {}",
            renderer.identifier(symbols.resolve(*name)),
            renderer.expression(value)
        )),
        StatementKind::Function {
            name,
            params,
            body: FunctionBody::Expression(body),
        } => {
            let params = params
                .iter()
                .map(|param| renderer.identifier(symbols.resolve(*param)))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "{}\\left({params}\\right) = {}",
                renderer.identifier(symbols.resolve(*name)),
                renderer.expression(body)
            ))
        }
        _ => None,
    }
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
        theme.ui.background.0,
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
    let stroke = egui::Stroke::new(2.4_f32, theme.ui.accent.0);
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
            .rect_filled(rect, 2.0, theme.ui.muted.0.gamma_multiply(0.12));
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            2.0,
            egui::Stroke::new(1.0_f32, theme.ui.accent.0),
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
            egui::Stroke::new(1.0_f32, theme.ui.accent.0),
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

struct OpenDocument {
    name: String,
    source: String,
    dirty: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct WorkspaceFile {
    path: String,
    source: String,
}

#[derive(Default, Serialize, Deserialize)]
struct WorkspaceState {
    folders: Vec<String>,
    files: Vec<WorkspaceFile>,
    active_file: Option<String>,
    selected_folder: String,
}

fn workspace_parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn workspace_label(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn valid_workspace_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    Output,
    Problems,
    References,
}

struct WebSeries {
    label: String,
    samples: Vec<(f64, Option<f64>)>,
    critical_points: Vec<CriticalPoint>,
}

#[derive(Clone, Copy, PartialEq)]
struct WebGraphProbe {
    series_index: usize,
    x: f64,
    y: f64,
    critical: Option<plot::CriticalKind>,
}

struct WebProblem {
    line: usize,
    column: usize,
    message: String,
}

struct WebApp {
    context: egui::Context,
    source: String,
    output: String,
    error: Option<String>,
    output_error: Option<String>,
    problems: Vec<WebProblem>,
    document: Option<Document>,
    tex: String,
    tab: Tab,
    documents: Vec<OpenDocument>,
    workspace: WorkspaceState,
    active_document: usize,
    pending_close: Option<usize>,
    reveal_tab: bool,
    plot_expression: String,
    plot_series: Vec<WebSeries>,
    plot_error: Option<String>,
    plot_bounds: [f64; 4],
    plot_svg: String,
    plot_custom_view: bool,
    plot_fit_y_pending: bool,
    plot_due: Option<f64>,
    plot_pending: bool,
    plot_force_due: bool,
    plot_sample_bounds: Option<[f64; 2]>,
    plot_probe: Option<WebGraphProbe>,
    repl_input: String,
    repl_preview_source: String,
    repl_preview_latex: Option<String>,
    repl_history: Vec<String>,
    repl_history_index: Option<usize>,
    last_repl_submission: String,
    queued_inputs: Vec<String>,
    generation: u64,
    run_generation: u64,
    preview_generation: u64,
    plot_generation: u64,
    repl_generation: u64,
    lsp_version: u64,
    pending: bool,
    analysis_started_at: Option<f64>,
    pending_pdf: bool,
    repl_pending: bool,
    due: Option<f64>,
    worker: Option<Worker>,
    _onmessage: Option<Closure<dyn FnMut(MessageEvent)>>,
    response_sender: Sender<String>,
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
    highlight_theme: usize,
    highlight_job: LayoutJob,
    editor_font_size: f32,
    themes: Vec<Theme>,
    selected_theme: usize,
    theme_path: String,
    font_path: String,
    line_numbers: LineNumberMode,
    auto_greek: bool,
    dirty: bool,
    settings_open: bool,
    save_as_open: bool,
    save_as_name: String,
    cursor_line: usize,
    cursor_column: usize,
    cursor_char_index: usize,
    rename_target: Option<String>,
    rename_input: String,
    references_target: Option<String>,
    hover_key: Option<usize>,
    hover_generation: u64,
    hover_since: f64,
    hover_value: Option<String>,
    first_frame: bool,
    reveal_cursor: bool,
    quick_open: bool,
    quick_open_query: String,
    explorer_root_open: bool,
    explorer_expanded: HashSet<String>,
}

impl WebApp {
    fn new(context: egui::Context) -> Self {
        let (sender, responses) = mpsc::channel();
        let response_sender = sender.clone();
        let (worker, onmessage, status) = match create_worker() {
            Ok(worker) => {
                let repaint_context = context.clone();
                let onmessage = Closure::wrap(Box::new(move |event: MessageEvent| {
                    if let Some(result) = event.data().as_string() {
                        if sender.send(result).is_ok() {
                            repaint_context.request_repaint();
                        }
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
        let workspace = storage
            .as_ref()
            .and_then(|storage| storage.get_item("scas.workspace.v1").ok().flatten())
            .and_then(|value| serde_json::from_str::<WorkspaceState>(&value).ok())
            .unwrap_or_default();
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
        let dirty = storage
            .as_ref()
            .and_then(|storage| storage.get_item("scas.draftDirty").ok().flatten())
            .map_or(stored.is_some(), |value| value == "true");
        let restored_file = if shared.is_none() {
            workspace
                .active_file
                .as_ref()
                .and_then(|path| workspace.files.iter().find(|file| &file.path == path))
                .cloned()
        } else {
            None
        };
        let source_name = if shared.is_some() {
            "shared.scas".to_owned()
        } else if let Some(file) = &restored_file {
            file.path.clone()
        } else {
            "Untitled".to_owned()
        };
        let source = shared
            .or_else(|| restored_file.as_ref().map(|file| file.source.clone()))
            .or(stored)
            .unwrap_or_default();
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
        let mut explorer_expanded = HashSet::new();
        let mut folder = workspace
            .active_file
            .as_deref()
            .map(workspace_parent)
            .unwrap_or(&workspace.selected_folder);
        while !folder.is_empty() {
            explorer_expanded.insert(folder.to_owned());
            folder = workspace_parent(folder);
        }
        let restored_workspace_file = restored_file.is_some();
        let mut app = Self {
            context,
            source: source.clone(),
            output: String::new(),
            error: None,
            output_error: None,
            problems: Vec::new(),
            document: initial_preview,
            tex: initial_tex,
            tab: Tab::Document,
            documents: vec![OpenDocument {
                name: source_name.clone(),
                source: source.clone(),
                dirty: if restored_workspace_file {
                    false
                } else {
                    dirty
                },
            }],
            workspace,
            active_document: 0,
            pending_close: None,
            reveal_tab: false,
            plot_expression: "sin(x)".to_owned(),
            plot_series: Vec::new(),
            plot_error: None,
            plot_bounds: [-10.0, 10.0, -1.0, 1.0],
            plot_svg: String::new(),
            plot_custom_view: false,
            plot_fit_y_pending: false,
            plot_due: None,
            plot_pending: false,
            plot_force_due: false,
            plot_sample_bounds: None,
            plot_probe: None,
            repl_input: String::new(),
            repl_preview_source: String::new(),
            repl_preview_latex: None,
            repl_history: Vec::new(),
            repl_history_index: None,
            last_repl_submission: String::new(),
            queued_inputs: Vec::new(),
            generation: 0,
            run_generation: 0,
            preview_generation: 0,
            plot_generation: 0,
            repl_generation: 0,
            lsp_version: 0,
            pending: false,
            analysis_started_at: None,
            pending_pdf: false,
            repl_pending: false,
            due: None,
            worker,
            _onmessage: onmessage,
            response_sender,
            responses,
            status,
            source_name,
            preview_json: initial_preview_json,
            explorer_open: true,
            preview_open: true,
            preview_width: None,
            output_open: false,
            bottom_tab: BottomTab::Output,
            highlight_source: String::new(),
            highlight_font_size_bits: 0,
            highlight_theme: usize::MAX,
            highlight_job: LayoutJob::default(),
            editor_font_size,
            themes,
            selected_theme,
            theme_path: String::new(),
            font_path: String::new(),
            line_numbers,
            auto_greek,
            dirty: if restored_workspace_file {
                false
            } else {
                dirty
            },
            settings_open: false,
            save_as_open: false,
            save_as_name: "document.scas".to_owned(),
            cursor_line: 1,
            cursor_column: 1,
            cursor_char_index: 0,
            rename_target: None,
            rename_input: String::new(),
            references_target: None,
            hover_key: None,
            hover_generation: 0,
            hover_since: 0.0,
            hover_value: None,
            first_frame: true,
            reveal_cursor: false,
            quick_open: false,
            quick_open_query: String::new(),
            explorer_root_open: true,
            explorer_expanded,
        };
        app.request("preview");
        app
    }

    fn load_source(&mut self, source: String) {
        self.load_source_named("imported.scas".to_owned(), source);
    }

    fn load_source_named(&mut self, name: String, source: String) {
        let name = if name.is_empty() {
            "imported.scas".to_owned()
        } else {
            name
        };
        if let Some(index) = self
            .documents
            .iter()
            .position(|document| document.name == name)
        {
            self.select_document(index);
            return;
        }
        if !self.workspace.files.iter().any(|file| file.path == name) {
            self.workspace.files.push(WorkspaceFile {
                path: name.clone(),
                source: source.clone(),
            });
        }
        let parent = workspace_parent(&name);
        if parent.is_empty() || self.workspace.folders.iter().any(|folder| folder == parent) {
            self.workspace.selected_folder = parent.to_owned();
            self.explorer_expanded.insert(parent.to_owned());
        }
        self.store_active_document();
        self.source = source;
        self.source_name = name;
        self.documents.push(OpenDocument {
            name: self.source_name.clone(),
            source: self.source.clone(),
            dirty: false,
        });
        self.active_document = self.documents.len() - 1;
        self.reveal_tab = true;
        self.tab = Tab::Document;
        self.dirty = false;
        self.highlight_source.clear();
        self.preview_json = "[]".to_owned();
        self.tex.clear();
        self.document = None;
        self.error = None;
        self.save_draft();
        self.request("preview");
    }

    fn new_file(&mut self) {
        let parent = self.workspace.selected_folder.clone();
        self.new_file_in(&parent);
    }

    fn new_file_in(&mut self, parent: &str) {
        let Some(input) = prompt_input("New file name") else {
            return;
        };
        let input = input.trim();
        if !valid_workspace_name(input) {
            self.status = "Enter a file name without / or \\".to_owned();
            return;
        }
        let name = if input.ends_with(".scas") {
            input.to_owned()
        } else {
            format!("{input}.scas")
        };
        let path = if parent.is_empty() {
            name
        } else {
            format!("{parent}/{name}")
        };
        if self.workspace.files.iter().any(|file| file.path == path) {
            self.status = format!("{path} already exists");
            return;
        }
        self.workspace.selected_folder = parent.to_owned();
        self.explorer_root_open = true;
        self.explorer_expanded.insert(parent.to_owned());
        self.load_source_named(path, String::new());
    }

    fn new_folder_in(&mut self, parent: &str) {
        let Some(input) = prompt_input("New folder name") else {
            return;
        };
        let name = input.trim();
        if !valid_workspace_name(name) {
            self.status = "Enter a folder name without / or \\".to_owned();
            return;
        }
        let path = if parent.is_empty() {
            name.to_owned()
        } else {
            format!("{parent}/{name}")
        };
        if self.workspace.folders.iter().any(|folder| folder == &path) {
            self.status = format!("{path} already exists");
            return;
        }
        self.workspace.folders.push(path.clone());
        self.workspace.selected_folder = path;
        self.explorer_root_open = true;
        self.explorer_expanded.insert(parent.to_owned());
        self.save_draft();
    }

    fn store_active_document(&mut self) {
        if let Some(document) = self.documents.get_mut(self.active_document) {
            document.name.clone_from(&self.source_name);
            document.source.clone_from(&self.source);
            document.dirty = self.dirty;
        }
        if let Some(file) = self
            .workspace
            .files
            .iter_mut()
            .find(|file| file.path == self.source_name)
        {
            file.source.clone_from(&self.source);
        }
    }

    fn select_document(&mut self, index: usize) {
        if index == self.active_document || index >= self.documents.len() {
            self.tab = Tab::Document;
            return;
        }
        self.store_active_document();
        let document = &self.documents[index];
        self.source.clone_from(&document.source);
        self.source_name.clone_from(&document.name);
        self.dirty = document.dirty;
        self.active_document = index;
        let parent = workspace_parent(&self.source_name);
        if parent.is_empty() || self.workspace.folders.iter().any(|folder| folder == parent) {
            self.workspace.selected_folder = parent.to_owned();
            self.explorer_expanded.insert(parent.to_owned());
        }
        self.tab = Tab::Document;
        self.reveal_tab = true;
        self.highlight_source.clear();
        self.preview_json = "[]".to_owned();
        self.tex.clear();
        self.document = None;
        self.error = None;
        self.cursor_char_index = 0;
        self.cursor_line = 1;
        self.cursor_column = 1;
        self.save_draft();
        self.request("preview");
    }

    fn close_document(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        self.store_active_document();
        if self.documents[index].dirty {
            self.select_document(index);
            self.pending_close = Some(index);
        } else {
            self.close_document_unchecked(index);
        }
    }

    fn close_document_unchecked(&mut self, index: usize) {
        self.documents.remove(index);
        self.pending_close = None;
        if self.documents.is_empty() {
            self.documents.push(OpenDocument {
                name: "Untitled".to_owned(),
                source: String::new(),
                dirty: false,
            });
        }
        let next = if index == self.active_document {
            index.min(self.documents.len() - 1)
        } else if index < self.active_document {
            self.active_document - 1
        } else {
            self.active_document
        };
        let document = &self.documents[next];
        self.source.clone_from(&document.source);
        self.source_name.clone_from(&document.name);
        self.dirty = document.dirty;
        self.active_document = next;
        self.tab = Tab::Document;
        self.reveal_tab = true;
        self.highlight_source.clear();
        self.preview_json = "[]".to_owned();
        self.tex.clear();
        self.document = None;
        self.error = None;
        self.save_draft();
        self.request("preview");
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
            let _ = storage.set_item("scas.draftDirty", if self.dirty { "true" } else { "false" });
            let mut workspace = WorkspaceState {
                folders: self.workspace.folders.clone(),
                files: self.workspace.files.clone(),
                active_file: None,
                selected_folder: self.workspace.selected_folder.clone(),
            };
            for document in &self.documents {
                if let Some(file) = workspace
                    .files
                    .iter_mut()
                    .find(|file| file.path == document.name)
                {
                    file.source.clone_from(&document.source);
                }
            }
            if let Some(file) = workspace
                .files
                .iter_mut()
                .find(|file| file.path == self.source_name)
            {
                file.source.clone_from(&self.source);
                workspace.active_file = Some(self.source_name.clone());
            }
            if let Ok(value) = serde_json::to_string(&workspace) {
                let _ = storage.set_item("scas.workspace.v1", &value);
            }
        }
    }

    fn jump_to_source(&mut self, line: usize, column: usize) {
        let index = self
            .source
            .lines()
            .take(line.saturating_sub(1))
            .map(|line| line.chars().count() + 1)
            .sum::<usize>()
            + column.saturating_sub(1);
        let editor_id = egui::Id::new("web-source");
        let mut state = TextEdit::load_state(&self.context, editor_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(index.min(self.source.chars().count())),
            )));
        TextEdit::store_state(&self.context, editor_id, state);
        self.context
            .memory_mut(|memory| memory.request_focus(editor_id));
        self.reveal_cursor = true;
        self.cursor_char_index = index.min(self.source.chars().count());
        (self.cursor_line, self.cursor_column) =
            line_column_at_char(&self.source, self.cursor_char_index);
        self.tab = Tab::Document;
        if self.context.screen_rect().width() < 850.0 {
            self.preview_open = false;
        }
        self.status = format!(
            "Source line {}, column {}",
            self.cursor_line, self.cursor_column
        );
        self.context.request_repaint();
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

    fn request(&mut self, kind: &str) -> bool {
        if kind == "plot" && self.plot_pending {
            // The worker is single threaded. Keep just the latest desired view
            // instead of queuing every intermediate pan behind old samples.
            self.plot_due = Some(self.context.input(|input| input.time));
            self.plot_force_due = true;
            return true;
        }
        if kind == "run" {
            self.due = None;
            self.output_open = true;
            self.bottom_tab = BottomTab::Output;
        }
        self.generation = self.generation.wrapping_add(1);
        if matches!(kind, "run" | "preview") {
            self.lsp_version = self.lsp_version.wrapping_add(1);
            if let Some(worker) = &self.worker {
                let uri = "file:///browser.scas";
                let message = if self.lsp_version == 1 {
                    json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
                        "textDocument": {"uri": uri, "languageId": "scas", "version": self.lsp_version, "text": self.source}
                    }})
                } else {
                    json!({"jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
                        "textDocument": {"uri": uri, "version": self.lsp_version},
                        "contentChanges": [{"text": self.source}]
                    }})
                };
                let packet = json!({"kind": "lsp", "message": message});
                let _ = worker.post_message(&JsValue::from_str(&packet.to_string()));
            }
        }
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
                match kind {
                    "preview" => {
                        self.preview_generation = self.generation;
                        self.pending = true;
                        self.analysis_started_at = Some(self.context.input(|input| input.time));
                    }
                    "plot" => {
                        self.plot_generation = self.generation;
                        self.plot_pending = true;
                        self.plot_force_due = false;
                    }
                    "repl" | "repl_numeric" => {
                        self.repl_generation = self.generation;
                        self.repl_pending = true;
                    }
                    _ => {
                        self.run_generation = self.generation;
                        self.pending = true;
                        self.analysis_started_at = Some(self.context.input(|input| input.time));
                    }
                }
                if matches!(kind, "run" | "preview") {
                    self.context.request_repaint_after(Duration::from_secs_f64(
                        ANALYSIS_TIMEOUT_SECONDS + 0.1,
                    ));
                }
                (if kind == "plot" {
                    "Sampling plot…"
                } else {
                    "Evaluating…"
                })
                .clone_into(&mut self.status);
                return true;
            } else {
                "Could not send work to the browser worker".clone_into(&mut self.status);
            }
        } else {
            "Web Worker unavailable; execution disabled".clone_into(&mut self.status);
        }
        false
    }

    fn request_hover(&mut self, byte: usize) {
        // Cache an empty result while the worker runs so repaints cannot queue
        // the same expensive parse and evaluation again.
        self.hover_value = Some(String::new());
        self.generation = self.generation.wrapping_add(1);
        self.hover_generation = self.generation;
        if let Some(worker) = &self.worker {
            let packet = json!({
                "kind": "hover",
                "generation": self.hover_generation,
                "source": self.source,
                "byte": byte,
            });
            let _ = worker.post_message(&JsValue::from_str(&packet.to_string()));
        }
    }

    /// Hard cancellation for Wasm execution. A busy Web Worker cannot process
    /// an interrupt message, so termination and replacement is the only true
    /// cancellation boundary in browsers.
    fn interrupt_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.terminate();
        }
        self.generation = self.generation.wrapping_add(1);
        self.run_generation = self.generation;
        self.preview_generation = self.generation;
        self.plot_generation = self.generation;
        self.repl_generation = self.generation;
        self.lsp_version = 0;
        self.pending = false;
        self.analysis_started_at = None;
        self.pending_pdf = false;
        self.repl_pending = false;
        self.plot_due = None;
        self.plot_pending = false;
        self.plot_force_due = false;
        self.hover_generation = 0;
        self.hover_value = None;
        match create_worker() {
            Ok(worker) => {
                let sender = self.response_sender.clone();
                let repaint_context = self.context.clone();
                let onmessage = Closure::wrap(Box::new(move |event: MessageEvent| {
                    if let Some(result) = event.data().as_string() {
                        if sender.send(result).is_ok() {
                            repaint_context.request_repaint();
                        }
                    }
                }) as Box<dyn FnMut(_)>);
                worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
                self.worker = Some(worker);
                self._onmessage = Some(onmessage);
                self.status = "Execution cancelled; worker restarted".to_owned();
            }
            Err(_) => {
                self._onmessage = None;
                self.status = "Execution cancelled; worker restart failed".to_owned();
            }
        }
    }

    fn poll(&mut self, context: &egui::Context) {
        while let Ok(response) = self.responses.try_recv() {
            let Ok(value) = serde_json::from_str::<Value>(&response) else {
                continue;
            };
            let kind = value["kind"].as_str().unwrap_or("run");
            if kind == "hover" {
                if value["generation"].as_u64() == Some(self.hover_generation)
                    && value["byte"].as_u64() == self.hover_key.map(|byte| byte as u64)
                {
                    self.hover_value = Some(value["value"].as_str().unwrap_or("").to_owned());
                    context.request_repaint();
                }
                continue;
            }
            if kind == "lsp" {
                for notification in value["notifications"].as_array().into_iter().flatten() {
                    let params = &notification["params"];
                    if params["version"].as_u64() != Some(self.lsp_version) {
                        continue;
                    }
                    if let Some(diagnostics) = params["diagnostics"].as_array() {
                        self.problems = diagnostics
                            .iter()
                            .map(|diagnostic| WebProblem {
                                line: diagnostic["range"]["start"]["line"].as_u64().unwrap_or(0)
                                    as usize
                                    + 1,
                                column: diagnostic["range"]["start"]["character"]
                                    .as_u64()
                                    .unwrap_or(0) as usize
                                    + 1,
                                message: diagnostic["message"].as_str().unwrap_or("").to_owned(),
                            })
                            .collect();
                        context.request_repaint();
                    }
                }
                continue;
            }
            let latest = match kind {
                "repl" => self.repl_generation,
                "plot" => self.plot_generation,
                "preview" => self.preview_generation,
                _ => self.run_generation,
            };
            if value["generation"].as_u64() != Some(latest) {
                continue;
            }
            if kind == "repl" {
                self.repl_pending = false;
            } else if matches!(kind, "run" | "preview")
                && latest >= self.run_generation.max(self.preview_generation)
            {
                self.pending = false;
                self.analysis_started_at = None;
            }
            if let Some(prompt) = value["input_required"].as_str() {
                if let Some(answer) = prompt_input(prompt) {
                    self.queued_inputs.push(answer);
                    if kind == "repl" {
                        self.repl_input = self.last_repl_submission.clone();
                        self.request(if value["numeric"].as_bool() == Some(true) {
                            "repl_numeric"
                        } else {
                            "repl"
                        });
                        self.repl_input.clear();
                    } else {
                        self.request("run");
                    }
                    continue;
                }
            } else {
                self.queued_inputs.clear();
            }
            if kind == "repl" {
                receive_repl_result(&response);
                if value["input"].as_str() == Some(":reset") && value["error"].is_null() {
                    self.repl_history.clear();
                }
                self.status = if value["error"].is_null() {
                    "Ready"
                } else {
                    "Needs attention"
                }
                .to_owned();
                context.request_repaint();
                continue;
            }
            if kind == "plot" {
                self.plot_pending = false;
                if self.plot_force_due {
                    // A newer Plot, Fit y, or Reset command superseded this
                    // result while the worker was busy.
                    self.plot_due = Some(context.input(|input| input.time));
                    context.request_repaint();
                    continue;
                }
                self.plot_error = value["error"].as_str().map(str::to_owned);
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
                        .map(|series| {
                            let samples: Vec<(f64, Option<f64>)> = series["samples"]
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
                                .unwrap_or_default();
                            let critical_points = plot::detect_critical_points(&samples);
                            WebSeries {
                                label: series["label"].as_str().unwrap_or("curve").to_owned(),
                                samples,
                                critical_points,
                            }
                        })
                        .collect();
                    self.plot_sample_bounds =
                        value["sample_bounds"].as_array().and_then(|bounds| {
                            Some([bounds.first()?.as_f64()?, bounds.get(1)?.as_f64()?])
                        });
                    self.plot_probe = self
                        .plot_probe
                        .and_then(|probe| self.plot_series.get(probe.series_index).map(|_| probe));
                }
                if let Some(svg) = value["svg"].as_str() {
                    svg.clone_into(&mut self.plot_svg);
                }
                self.status = if self.plot_error.is_some() {
                    "Needs attention"
                } else {
                    "Ready"
                }
                .to_owned();
                if self.plot_due.is_some()
                    && (self.plot_force_due
                        || (self.plot_error.is_none() && self.plot_needs_samples()))
                {
                    self.plot_due = Some(context.input(|input| input.time));
                } else {
                    self.plot_due = None;
                    self.plot_force_due = false;
                }
                context.request_repaint();
                continue;
            }
            if kind == "run" {
                self.output = value["output"].as_str().unwrap_or("").to_owned();
                self.output_error = value["error"].as_str().map(str::to_owned);
            }
            if latest < self.run_generation.max(self.preview_generation) {
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
                    .map(|series| {
                        let samples: Vec<(f64, Option<f64>)> = series["samples"]
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
                            .unwrap_or_default();
                        let critical_points = plot::detect_critical_points(&samples);
                        WebSeries {
                            label: series["label"].as_str().unwrap_or("curve").to_owned(),
                            samples,
                            critical_points,
                        }
                    })
                    .collect();
                self.plot_probe = self
                    .plot_probe
                    .and_then(|probe| self.plot_series.get(probe.series_index).map(|_| probe));
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
            if self.pending_pdf {
                self.pending_pdf = false;
                if self.error.is_none() {
                    print_document(&self.preview_json, &self.tex);
                    self.status = "Choose Save as PDF in the print dialog".to_owned();
                } else {
                    self.status = "Fix document errors before exporting PDF".to_owned();
                }
            }
            context.request_repaint();
        }
    }

    fn plot_view(&self) -> PlotBounds {
        let [x_min, x_max, y_min, y_max] = self.plot_bounds;
        PlotBounds::new(x_min, x_max, y_min, y_max)
    }

    fn plot_needs_samples(&self) -> bool {
        let Some([sample_min, sample_max]) = self.plot_sample_bounds else {
            return true;
        };
        let [view_min, view_max, _, _] = self.plot_bounds;
        let margin = (view_max - view_min) * 0.1;
        sample_min > view_min - margin || sample_max < view_max + margin
    }

    fn apply_plot_view(&mut self, view: PlotBounds, now: f64, context: &egui::Context) {
        self.plot_bounds = [view.x_min, view.x_max, view.y_min, view.y_max];
        self.plot_custom_view = true;
        self.plot_fit_y_pending = false;
        self.plot_svg.clear();
        if self.plot_needs_samples() {
            self.plot_due = Some(now);
            context.request_repaint();
        } else {
            self.plot_due = None;
        }
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
        let (canvas_rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        // Keep tick labels and interaction hints outside the data area.
        let rect = egui::Rect::from_min_max(
            canvas_rect.min + egui::vec2(38.0, 12.0),
            canvas_rect.max - egui::vec2(12.0, 38.0),
        );
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
        let (pinch_zoom, pointer) = ui.input(|input| {
            (
                input.zoom_delta(),
                input.pointer.hover_pos().unwrap_or(rect.center()),
            )
        });
        if response.hovered() && (pinch_zoom - 1.0).abs() > f32::EPSILON {
            // A larger pinch delta means zooming in, while PlotBounds uses a
            // smaller span factor for zooming in. Ordinary two-finger wheel
            // motion is intentionally ignored.
            let factor = (1.0 / f64::from(pinch_zoom)).clamp(0.5, 2.0);
            let x_ratio = f64::from((pointer.x - rect.left()) / rect.width().max(1.0));
            let y_ratio = f64::from((pointer.y - rect.top()) / rect.height().max(1.0));
            self.zoom_plot(factor, x_ratio, y_ratio, now, ui.ctx());
        }
        let painter = ui.painter_at(canvas_rect);
        painter.rect_filled(canvas_rect, 0.0, theme.ui.background.0);
        let [x_min, x_max, y_min, y_max] = self.plot_bounds;
        let map = |x: f64, y: f64| {
            egui::pos2(
                rect.left() + ((x - x_min) / (x_max - x_min)) as f32 * rect.width(),
                rect.bottom() - ((y - y_min) / (y_max - y_min)) as f32 * rect.height(),
            )
        };
        let x_step = plot::nice_step(
            x_max - x_min,
            f64::from((rect.width() / 85.0).clamp(2.0, 12.0)),
        );
        let y_step = plot::nice_step(
            y_max - y_min,
            f64::from((rect.height() / 65.0).clamp(2.0, 10.0)),
        );
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
                let stroke = egui::Stroke::new(if zero { 1.4_f32 } else { 1.0_f32 }, color);
                if vertical {
                    let x = map(value, 0.0).x;
                    painter.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        stroke,
                    );
                    painter.text(
                        egui::pos2(x, rect.bottom() + 6.0),
                        egui::Align2::CENTER_TOP,
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
                        egui::pos2(rect.left() - 6.0, y),
                        egui::Align2::RIGHT_CENTER,
                        format_plot_tick(value, step),
                        FontId::monospace(10.0),
                        theme.ui.muted.0,
                    );
                }
                value += step;
            }
        }
        let curve_painter = painter.with_clip_rect(rect);
        let colors = graph_colors(theme);
        for (index, series) in self.plot_series.iter().enumerate() {
            let mut previous = None;
            for &(x, y) in &series.samples[visible_plot_samples(&series.samples, x_min, x_max)] {
                let point = y.filter(|y| y.is_finite()).map(|y| map(x, y));
                if let (Some(a), Some(b)) = (previous, point) {
                    curve_painter.line_segment(
                        [a, b],
                        egui::Stroke::new(2.0_f32, colors[index % colors.len()]),
                    );
                }
                previous = point;
            }
            let color = colors[index % colors.len()];
            for point in &series.critical_points {
                if point.x < x_min || point.x > x_max {
                    continue;
                }
                let center = map(point.x, point.y);
                match point.kind {
                    plot::CriticalKind::Hole => {
                        curve_painter.circle_stroke(center, 4.5, egui::Stroke::new(1.6_f32, color));
                    }
                    _ => {
                        curve_painter.circle_filled(center, 3.6, color);
                        curve_painter.circle_stroke(
                            center,
                            5.2,
                            egui::Stroke::new(1.0_f32, theme.ui.background.0),
                        );
                    }
                }
            }
        }
        let hovered_probe = response.hover_pos().and_then(|pointer| {
            snap_web_graph_probe(pointer, &self.plot_bounds, rect, &self.plot_series, 12.0)
        });
        if response.clicked() {
            self.plot_probe = hovered_probe;
        }
        if let Some(probe) = self.plot_probe.or(hovered_probe) {
            draw_web_graph_probe(
                &painter,
                rect,
                &self.plot_bounds,
                probe,
                colors[probe.series_index % colors.len()],
                theme,
            );
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
            egui::pos2(canvas_rect.right() - 12.0, canvas_rect.bottom() - 4.0),
            egui::Align2::RIGHT_BOTTOM,
            if rect.width() > 600.0 {
                "Drag to pan · Pinch to zoom · Hover a point · Click to pin"
            } else {
                "Drag to pan · Pinch to zoom"
            },
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

fn plot_screen_point(bounds: &[f64; 4], rect: egui::Rect, x: f64, y: f64) -> egui::Pos2 {
    let [x_min, x_max, y_min, y_max] = *bounds;
    egui::pos2(
        rect.left() + ((x - x_min) / (x_max - x_min)) as f32 * rect.width(),
        rect.bottom() - ((y - y_min) / (y_max - y_min)) as f32 * rect.height(),
    )
}

fn visible_plot_samples(
    samples: &[(f64, Option<f64>)],
    x_min: f64,
    x_max: f64,
) -> std::ops::Range<usize> {
    // Include one point on either side so lines crossing the clip boundary
    // remain connected. Binary search avoids walking prefetched offscreen data.
    let start = samples
        .partition_point(|(x, _)| *x < x_min)
        .saturating_sub(1);
    let end = (samples.partition_point(|(x, _)| *x <= x_max) + 1).min(samples.len());
    start..end
}

fn snap_web_graph_probe(
    pointer: egui::Pos2,
    bounds: &[f64; 4],
    rect: egui::Rect,
    series: &[WebSeries],
    max_distance: f32,
) -> Option<WebGraphProbe> {
    let mut best: Option<(f32, WebGraphProbe)> = None;
    let [x_min, x_max, _, _] = *bounds;
    for (series_index, curve) in series.iter().enumerate() {
        for point in &curve.critical_points {
            if point.x < x_min || point.x > x_max {
                continue;
            }
            let distance = pointer.distance(plot_screen_point(bounds, rect, point.x, point.y));
            if distance <= max_distance
                && best
                    .as_ref()
                    .map_or(true, |(best_distance, _)| distance < *best_distance)
            {
                best = Some((
                    distance,
                    WebGraphProbe {
                        series_index,
                        x: point.x,
                        y: point.y,
                        critical: Some(point.kind),
                    },
                ));
            }
        }
    }
    if best.is_some() {
        return best.map(|(_, probe)| probe);
    }
    for (series_index, curve) in series.iter().enumerate() {
        for &(x, y) in &curve.samples[visible_plot_samples(&curve.samples, x_min, x_max)] {
            let Some(y) = y.filter(|value| value.is_finite()) else {
                continue;
            };
            let distance = pointer.distance(plot_screen_point(bounds, rect, x, y));
            if distance <= max_distance
                && best
                    .as_ref()
                    .map_or(true, |(best_distance, _)| distance < *best_distance)
            {
                best = Some((
                    distance,
                    WebGraphProbe {
                        series_index,
                        x,
                        y,
                        critical: None,
                    },
                ));
            }
        }
    }
    best.map(|(_, probe)| probe)
}

fn draw_web_graph_probe(
    painter: &egui::Painter,
    rect: egui::Rect,
    bounds: &[f64; 4],
    probe: WebGraphProbe,
    color: Color32,
    theme: &Theme,
) {
    let screen = plot_screen_point(bounds, rect, probe.x, probe.y);
    let guide = egui::Stroke::new(1.0_f32, color.gamma_multiply(0.55));
    painter.line_segment(
        [
            egui::pos2(screen.x, rect.top()),
            egui::pos2(screen.x, rect.bottom()),
        ],
        guide,
    );
    painter.line_segment(
        [
            egui::pos2(rect.left(), screen.y),
            egui::pos2(rect.right(), screen.y),
        ],
        guide,
    );
    painter.circle_filled(screen, 4.5, color);
    painter.circle_stroke(screen, 6.0, egui::Stroke::new(1.5_f32, theme.ui.text.0));
    let kind = match probe.critical {
        Some(plot::CriticalKind::Zero) => "zero · ",
        Some(plot::CriticalKind::Hole) => "hole · ",
        Some(plot::CriticalKind::Maximum) => "maximum · ",
        Some(plot::CriticalKind::Minimum) => "minimum · ",
        None => "",
    };
    let label = format!("{kind}x = {:.5}, y = {:.5}", probe.x, probe.y);
    let galley = painter.layout_no_wrap(label, FontId::monospace(11.0), theme.contrast_on_accent());
    let mut label_rect = egui::Rect::from_min_size(
        screen + egui::vec2(10.0, -galley.size().y - 9.0),
        galley.size() + egui::vec2(12.0, 8.0),
    );
    if label_rect.right() > rect.right() - 4.0 {
        label_rect = label_rect.translate(egui::vec2(rect.right() - 4.0 - label_rect.right(), 0.0));
    }
    if label_rect.top() < rect.top() + 4.0 {
        label_rect = label_rect.translate(egui::vec2(0.0, rect.top() + 4.0 - label_rect.top()));
    }
    painter.rect_filled(label_rect, 2.0, theme.ui.accent.0);
    painter.galley(
        label_rect.min + egui::vec2(6.0, 4.0),
        galley,
        theme.contrast_on_accent(),
    );
}

impl WebApp {
    fn submit_repl(&mut self) {
        self.submit_repl_kind("repl");
    }

    fn submit_repl_numerically(&mut self) {
        if self.repl_input.trim_start().starts_with(':') {
            self.status =
                "Ctrl+Enter numerically evaluates expressions; use Enter for commands".to_owned();
            return;
        }
        self.submit_repl_kind("repl_numeric");
    }

    fn submit_repl_kind(&mut self, kind: &str) {
        if self.repl_pending || self.repl_input.trim().is_empty() {
            return;
        }
        self.repl_input = self.repl_input.trim().to_owned();
        self.last_repl_submission = self.repl_input.clone();
        if self.request(kind) {
            self.repl_history.push(self.repl_input.clone());
            self.repl_input.clear();
            self.repl_history_index = None;
        }
    }

    fn show_repl(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        if self.repl_preview_source != self.repl_input {
            self.repl_preview_latex = repl_expression_latex(&self.repl_input);
            self.repl_preview_source.clone_from(&self.repl_input);
        }
        let (enter_pressed, history_up, history_down) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::Enter)
                    && !input.modifiers.shift
                    && !input.modifiers.ctrl
                    && !input.modifiers.command,
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
            )
        });
        egui::Frame::none()
            .fill(theme.ui.panel.0)
            .inner_margin(egui::Margin::symmetric(16.0, 8.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(30.0);
                    ui.label(
                        RichText::new("REPL")
                            .color(theme.ui.text.0)
                            .strong()
                            .size(16.0),
                    );
                    if self.repl_pending {
                        ui.spinner();
                        ui.label(RichText::new("Evaluating…").color(theme.ui.muted.0));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.repl_pending, egui::Button::new("Reset session"))
                            .clicked()
                        {
                            ":reset".clone_into(&mut self.repl_input);
                            self.submit_repl();
                        }
                    });
                });
            });
        let muted = theme.ui.muted.0;
        let narrow_repl = ui.available_width() < 600.0;
        let completion_count =
            editor_assist::completions(&self.repl_input, self.repl_input.chars().count()).len;
        let composer_height = if narrow_repl && completion_count > 0 {
            318.0
        } else if narrow_repl {
            206.0
        } else if completion_count > 0 {
            248.0
        } else {
            192.0
        };
        let history_height = (ui.available_height() - composer_height).max(32.0);
        let (history_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), history_height),
            egui::Sense::hover(),
        );
        show_repl_surface(
            "history",
            "",
            history_rect.left(),
            history_rect.top(),
            history_rect.width(),
            history_rect.height(),
            ui.ctx().pixels_per_point(),
        );

        ui.add_space(8.0);
        let input_id = egui::Id::new("web-repl-input");
        let mut submit = false;
        egui::Frame::none()
            .fill(theme.ui.panel.0)
            .inner_margin(egui::Margin::symmetric(12.0, 12.0))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Expression").color(theme.ui.text.0).strong().size(13.0));
                    ui.label(
                        RichText::new(
                            "Enter to evaluate · Ctrl+Enter for a decimal · Up/Down for history",
                        )
                        .size(12.0)
                        .color(muted),
                    );
                });
                ui.add_space(5.0);
                let (math_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 66.0), egui::Sense::hover());
                let preview = json!({"latex": self.repl_preview_latex, "input": self.repl_input});
                show_repl_surface("entry", &preview.to_string(), math_rect.left(), math_rect.top(),
                    math_rect.width(), math_rect.height(), ui.ctx().pixels_per_point());
                ui.add_space(4.0);
                let response = ui.add(
                    TextEdit::singleline(&mut self.repl_input)
                        .id(input_id)
                        .font(FontId::monospace(16.0))
                        .desired_width(f32::INFINITY)
                        .hint_text("integral(sin(x), x)"),
                );
                if response.changed() {
                    if self.auto_greek {
                        expand_greek_shortcuts(&mut self.repl_input);
                    }
                    self.repl_history_index = None;
                }
                let repl_completions = editor_assist::completions(
                    &self.repl_input,
                    self.repl_input.chars().count(),
                );
                let mut chosen_completion = None;
                if repl_completions.len > 0 {
                    ui.add_space(5.0);
                    ui.label(
                        RichText::new("Suggestions")
                            .size(11.0)
                            .strong()
                            .color(muted),
                    );
                    ui.horizontal_wrapped(|ui| {
                        for completion in repl_completions.iter() {
                            if ui
                                .add_sized(
                                    [132.0, if narrow_repl { 44.0 } else { 30.0 }],
                                    egui::Button::new(completion.label),
                                )
                                .on_hover_text(completion.detail)
                                .clicked()
                            {
                                chosen_completion = Some(completion);
                            }
                        }
                    });
                }
                if let Some(completion) = chosen_completion {
                    let insertion = completion.insertion.text().replace("$0", "");
                    let start = char_to_byte(&self.repl_input, repl_completions.start_char);
                    self.repl_input.replace_range(start.., &insertion);
                    self.repl_history_index = None;
                    response.request_focus();
                }
                if response.has_focus() {
                    let tab = ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                    });
                    if (history_up || history_down) && !self.repl_history.is_empty() {
                        let cursor = self
                            .repl_history_index
                            .unwrap_or(self.repl_history.len());
                        let next = if history_up {
                            cursor.saturating_sub(1)
                        } else {
                            (cursor + 1).min(self.repl_history.len())
                        };
                        if next == self.repl_history.len() {
                            self.repl_input.clear();
                            self.repl_history_index = None;
                        } else {
                            self.repl_input.clone_from(&self.repl_history[next]);
                            self.repl_history_index = Some(next);
                        }
                    }
                    if tab {
                        if let Some(completion) = repl_completions.first() {
                            let insertion = completion.insertion.text().replace("$0", "");
                            let start =
                                char_to_byte(&self.repl_input, repl_completions.start_char);
                            self.repl_input.replace_range(start.., &insertion);
                        }
                    }
                    submit = enter_pressed;
                }
                if enter_pressed && response.lost_focus() {
                    submit = true;
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.repl_pending && !self.repl_input.trim().is_empty(),
                            egui::Button::new(RichText::new("Evaluate").color(theme.contrast_on_accent()))
                                .fill(theme.ui.accent.0)
                                .min_size(egui::vec2(92.0, 30.0)),
                        )
                        .clicked()
                    {
                        submit = true;
                    }
                    ui.menu_button("Commands", |ui| {
                        ui.label("Enter a command in the expression field:");
                        ui.monospace(":help\n:precision N\n:plot sin(x)\n:definitions\n:history\n:clear\n:reset");
                    });
                });
            });
        if submit {
            self.submit_repl();
            self.context
                .memory_mut(|memory| memory.request_focus(input_id));
            self.context.request_repaint();
        }
    }

    fn preview_panel(&self, ui: &mut egui::Ui, context: &egui::Context) {
        let rect = ui.max_rect();
        show_preview(
            &self.preview_json,
            &self.tex,
            self.error.as_deref().unwrap_or(""),
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
                let response = ui.add_sized(
                    [(context.screen_rect().width() - 48.0).min(420.0), 25.0],
                    TextEdit::singleline(&mut self.save_as_name),
                );
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
                if !valid_workspace_name(trimmed) {
                    self.status = "Enter a file name without / or \\".to_owned();
                    return;
                }
                let name = if trimmed.ends_with(".scas") {
                    trimmed.to_owned()
                } else {
                    format!("{trimmed}.scas")
                };
                let parent = if self
                    .workspace
                    .files
                    .iter()
                    .any(|file| file.path == self.source_name)
                {
                    workspace_parent(&self.source_name)
                } else {
                    &self.workspace.selected_folder
                };
                let path = if parent.is_empty() {
                    name.clone()
                } else {
                    format!("{parent}/{name}")
                };
                if self
                    .workspace
                    .files
                    .iter()
                    .any(|file| file.path == path && file.path != self.source_name)
                {
                    self.status = format!("{path} already exists");
                    return;
                }
                if let Some(file) = self
                    .workspace
                    .files
                    .iter_mut()
                    .find(|file| file.path == self.source_name)
                {
                    file.path.clone_from(&path);
                    file.source.clone_from(&self.source);
                } else {
                    self.workspace.files.push(WorkspaceFile {
                        path: path.clone(),
                        source: self.source.clone(),
                    });
                }
                download_file(&name, &self.source, "text/plain");
                self.source_name = path.clone();
                if let Some(document) = self.documents.get_mut(self.active_document) {
                    document.name = path.clone();
                    document.source.clone_from(&self.source);
                }
                self.dirty = false;
                self.save_draft();
                self.status = format!("Saved {path}");
                self.save_as_open = false;
                if let Some(index) = self.pending_close.take() {
                    self.close_document_unchecked(index);
                    return;
                }
            }
        }
    }

    fn show_close_document(&mut self, context: &egui::Context) {
        let Some(index) = self.pending_close else {
            return;
        };
        if self.save_as_open {
            return;
        }
        egui::Window::new("Unsaved file")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.label(format!(
                    "Save changes to {} before closing?",
                    self.source_name
                ));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        if self.source_name.ends_with(".scas") {
                            download_file(
                                workspace_label(&self.source_name),
                                &self.source,
                                "text/plain",
                            );
                            self.dirty = false;
                            self.close_document_unchecked(index);
                        } else {
                            self.save_as_name = "document.scas".to_owned();
                            self.save_as_open = true;
                        }
                    }
                    if ui.button("Discard").clicked() {
                        self.close_document_unchecked(index);
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_close = None;
                    }
                });
            });
    }

    fn begin_rename(&mut self) {
        let byte = char_to_byte(&self.source, self.cursor_char_index);
        let index = SourceIndex::new(&self.source);
        if let Some(occurrence) = index
            .at(byte)
            .or_else(|| byte.checked_sub(1).and_then(|before| index.at(before)))
        {
            self.rename_input.clone_from(&occurrence.name);
            self.rename_target = Some(occurrence.name.clone());
        } else {
            self.status = "Place the cursor on a document symbol to rename it".to_owned();
        }
    }

    fn symbol_at_cursor(&self) -> Option<String> {
        let byte = char_to_byte(&self.source, self.cursor_char_index);
        let index = SourceIndex::new(&self.source);
        index
            .at(byte)
            .or_else(|| byte.checked_sub(1).and_then(|before| index.at(before)))
            .map(|occurrence| occurrence.name.clone())
    }

    fn insert_editor_template(
        &mut self,
        editor_id: egui::Id,
        start_char: usize,
        end_char: usize,
        template: &str,
    ) {
        let source_chars = self.source.chars().count();
        let start_char = start_char.min(source_chars);
        let end_char = end_char.clamp(start_char, source_chars);
        let marker = template.find("$0");
        let cursor_offset = marker
            .map(|index| template[..index].chars().count())
            .unwrap_or_else(|| template.chars().count());
        let insertion = marker.map_or_else(
            || template.to_owned(),
            |index| format!("{}{}", &template[..index], &template[index + 2..]),
        );
        let start_byte = char_to_byte(&self.source, start_char);
        let end_byte = char_to_byte(&self.source, end_char);
        self.source.replace_range(start_byte..end_byte, &insertion);
        self.cursor_char_index = start_char + cursor_offset;
        let mut state = TextEdit::load_state(&self.context, editor_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(self.cursor_char_index),
            )));
        TextEdit::store_state(&self.context, editor_id, state);
        self.context
            .memory_mut(|memory| memory.request_focus(editor_id));
        self.highlight_source.clear();
        self.dirty = true;
        self.save_draft();
        self.due = Some(self.context.input(|input| input.time) + 0.4);
        self.context
            .request_repaint_after(Duration::from_millis(420));
    }

    fn editor_selection_is_empty(&self, editor_id: egui::Id) -> bool {
        TextEdit::load_state(&self.context, editor_id)
            .and_then(|state| state.cursor.char_range())
            .map_or(true, |range| range.primary.index == range.secondary.index)
    }

    fn delete_editor_line(&mut self, editor_id: egui::Id) {
        let cursor = TextEdit::load_state(&self.context, editor_id)
            .and_then(|state| state.cursor.char_range())
            .map_or(self.cursor_char_index, |range| range.primary.index)
            .min(self.source.chars().count());
        let cursor_byte = char_to_byte(&self.source, cursor);
        let mut start = self.source[..cursor_byte]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let end = self.source[cursor_byte..]
            .find('\n')
            .map_or(self.source.len(), |index| cursor_byte + index + 1);

        // Removing the final line also removes its separator so the previous
        // line does not gain an unexpected blank line.
        if end == self.source.len() && start > 0 {
            start -= 1;
            if start > 0 && self.source.as_bytes()[start - 1] == b'\r' {
                start -= 1;
            }
        }

        self.source.replace_range(start..end, "");
        self.cursor_char_index = self.source[..start].chars().count();
        let mut state = TextEdit::load_state(&self.context, editor_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(self.cursor_char_index),
            )));
        TextEdit::store_state(&self.context, editor_id, state);
        self.context
            .memory_mut(|memory| memory.request_focus(editor_id));
        (self.cursor_line, self.cursor_column) =
            line_column_at_char(&self.source, self.cursor_char_index);
        self.highlight_source.clear();
        self.hover_key = None;
        self.dirty = true;
        self.save_draft();
        self.due = Some(self.context.input(|input| input.time) + 0.4);
        self.reveal_cursor = true;
        self.status = "Deleted current line".to_owned();
        self.context
            .request_repaint_after(Duration::from_millis(420));
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
                self.request("preview");
                self.status = format!(
                    "Renamed {target} to {} ({count} locations)",
                    self.rename_input.trim()
                );
            } else {
                self.status = "That symbol cannot be renamed unambiguously".to_owned();
            }
        }
    }

    fn workspace_tree(&mut self, ui: &mut egui::Ui, parent: &str, theme: &Theme) {
        let mut folders: Vec<String> = self
            .workspace
            .folders
            .iter()
            .filter(|path| workspace_parent(path) == parent)
            .cloned()
            .collect();
        folders.sort_by_key(|path| path.to_lowercase());
        for path in folders {
            let expanded = self.explorer_expanded.contains(&path);
            ui.horizontal(|ui| {
                if web_tree_disclosure(ui, !expanded, theme).clicked() {
                    if expanded {
                        self.explorer_expanded.remove(&path);
                    } else {
                        self.explorer_expanded.insert(path.clone());
                    }
                }
                ui.label(
                    RichText::new('\u{ea83}')
                        .font(FontId::new(
                            15.0,
                            FontFamily::Name("VS Code Codicons".into()),
                        ))
                        .color(theme.ui.muted.0),
                );
                if ui
                    .selectable_label(
                        self.workspace.selected_folder == path,
                        workspace_label(&path),
                    )
                    .clicked()
                {
                    self.workspace.selected_folder = path.clone();
                    self.explorer_expanded.insert(path.clone());
                    self.save_draft();
                }
                if web_icon_button(ui, '\u{ea7f}', "Add file to this folder", theme).clicked() {
                    self.new_file_in(&path);
                }
            });
            if self.explorer_expanded.contains(&path) {
                ui.indent(format!("workspace-{path}"), |ui| {
                    self.workspace_tree(ui, &path, theme)
                });
            }
        }
        let mut files: Vec<WorkspaceFile> = self
            .workspace
            .files
            .iter()
            .filter(|file| workspace_parent(&file.path) == parent)
            .cloned()
            .collect();
        files.sort_by_key(|file| file.path.to_lowercase());
        for file in files {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                let selected = self.tab == Tab::Document && self.source_name == file.path;
                if ui
                    .selectable_label(selected, workspace_label(&file.path))
                    .on_hover_text(&file.path)
                    .clicked()
                {
                    self.load_source_named(file.path, file.source);
                }
            });
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
                if !self.source_name.ends_with(".scas") {
                    self.save_as_name = "document.scas".to_owned();
                    self.save_as_open = true;
                } else {
                    download_file(
                        workspace_label(&self.source_name),
                        &self.source,
                        "text/plain",
                    );
                    self.dirty = false;
                    self.save_draft();
                    self.status = format!("Saved {}", self.source_name);
                }
                ui.close_menu();
            }
            if ui.button("Save As…  Ctrl+Shift+S").clicked() {
                self.save_as_name = if self.source_name.ends_with(".scas") {
                    workspace_label(&self.source_name).to_owned()
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
                if self.pending || self.due.is_some() {
                    self.pending_pdf = self.request("preview");
                    self.due = None;
                } else if self.error.is_some() {
                    self.status = "Fix document errors before exporting PDF".to_owned();
                } else {
                    print_document(&self.preview_json, &self.tex);
                    self.status = "Choose Save as PDF in the print dialog".to_owned();
                }
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

impl WebApp {
    fn show_document_toolbar(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        let panel = theme.ui.panel.0;
        egui::Frame::none().fill(panel).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                    if web_icon_button(ui, '\u{ebf4}', "Toggle mathematical preview", &theme)
                        .clicked()
                    {
                        self.preview_open = !self.preview_open;
                    }
                    if web_icon_button(ui, '\u{eb2c}', "Run active workspace · Ctrl+Enter", &theme)
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
                            ("Function", "block function", "fn name(args) {\n    $0\n}"),
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
                            ("Document RREF", "show every row operation", "rref($0) =>;"),
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
                });
            });
        });
    }
}

impl eframe::App for WebApp {
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let prior_tab = self.tab;
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
            || self.highlight_theme != self.selected_theme
        {
            if self.highlight_source != self.source {
                self.hover_key = None;
                self.hover_generation = 0;
                self.hover_value = None;
            }
            self.highlight_job = highlight_source(&self.source, editor_font.clone(), &theme);
            self.highlight_source.clone_from(&self.source);
            self.highlight_font_size_bits = effective_editor_font_size.to_bits();
            self.highlight_theme = self.selected_theme;
        }
        let now = context.input(|input| input.time);
        if self.pending
            && self
                .analysis_started_at
                .is_some_and(|started| now - started >= ANALYSIS_TIMEOUT_SECONDS)
        {
            self.interrupt_worker();
            self.status =
                "Live analysis timed out; worker restarted. Editing remains available.".to_owned();
        }
        if self.due.is_some_and(|due| now >= due) {
            if self.pending {
                // Keep only the newest edit waiting for live analysis. Posting
                // every intermediate buffer to the single worker can otherwise
                // build an unbounded queue and make the IDE appear frozen.
                self.due = Some(now + 0.1);
                context.request_repaint_after(Duration::from_millis(110));
            } else {
                self.due = None;
                self.request("preview");
            }
        }
        if self.plot_due.is_some_and(|due| now >= due) {
            if !self.plot_pending {
                self.plot_due = None;
                self.request("plot");
            }
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
        let editor_id = egui::Id::new("web-source");
        let editor_focused = context.memory(|memory| memory.has_focus(editor_id));
        if editor_focused
            && self.editor_selection_is_empty(editor_id)
            && context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::X))
        {
            self.delete_editor_line(editor_id);
        }
        // egui ignores extra Shift for a NONE shortcut: consume Shift+F12 first.
        if editor_focused
            && context.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::F12))
        {
            self.references_target = self.symbol_at_cursor();
            self.bottom_tab = BottomTab::References;
            self.output_open = true;
        }
        if editor_focused
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F12))
        {
            if let Some(name) = self.symbol_at_cursor() {
                let index = SourceIndex::new(&self.source);
                if let Some(definition) = index.definition(&name) {
                    self.jump_to_source(definition.line, definition.column);
                } else {
                    self.status = "No unambiguous definition in this document".to_owned();
                }
            }
        }
        if editor_focused
            && context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F2))
        {
            self.begin_rename();
        }
        if context.input_mut(|input| {
            input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::S,
            )
        }) {
            self.save_as_name = if self.source_name.ends_with(".scas") {
                workspace_label(&self.source_name).to_owned()
            } else {
                "document.scas".to_owned()
            };
            self.save_as_open = true;
        } else if context
            .input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
        {
            if !self.source_name.ends_with(".scas") {
                self.save_as_open = true;
            } else {
                download_file(
                    workspace_label(&self.source_name),
                    &self.source,
                    "text/plain",
                );
                self.dirty = false;
                self.save_draft();
                self.status = format!("Saved {}", self.source_name);
            }
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)) {
            if self.tab == Tab::Repl {
                self.submit_repl_numerically();
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
        let border = egui::Stroke::new(1.0_f32, muted.gamma_multiply(0.28));
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
            .exact_height(if narrow { 44.0 } else { 38.0 })
            .frame(
                egui::Frame::none()
                    .fill(theme.title_bar_color())
                    .inner_margin(egui::Margin::symmetric(7.0, 4.0)),
            )
            .show(context, |ui| {
                let available = ui.available_width();
                let compact = available < 700.0;
                let left_width = if compact {
                    92.0
                } else {
                    (available * 0.32).clamp(410.0, 690.0)
                };
                let center_width = if compact {
                    (available - left_width - 42.0).max(80.0)
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
                            if compact {
                                return;
                            }
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
                                    if ui.button("Find References  Shift+F12").clicked() {
                                        self.references_target = self.symbol_at_cursor();
                                        self.bottom_tab = BottomTab::References;
                                        self.output_open = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("Editor").clicked() {
                                        self.tab = Tab::Document;
                                        ui.close_menu();
                                    }
                                });
                            }
                            ui.menu_button("Run", |ui| {
                                if (self.pending || self.repl_pending)
                                    && ui.button("Interrupt").clicked()
                                {
                                    self.interrupt_worker();
                                    ui.close_menu();
                                }
                                if ui.button("Run Active  Ctrl+Enter").clicked() {
                                    match self.tab {
                                        Tab::Document => {
                                            self.request("run");
                                            self.output_open = true;
                                        }
                                        Tab::Repl => self.submit_repl(),
                                        Tab::Plot => {
                                            self.request("plot");
                                        }
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
                                    egui::Button::new(
                                        RichText::new("scas").color(theme.ui.text.0).size(12.0),
                                    )
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
                        if compact {
                            if web_icon_button(ui, '\u{eaf0}', "Toggle Explorer", &theme).clicked()
                            {
                                self.explorer_open = !self.explorer_open;
                            }
                            return;
                        }
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
        if !narrow {
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
        }
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
                        ui.label(
                            RichText::new("Explorer")
                                .color(theme.ui.text.0)
                                .strong()
                                .size(13.0),
                        );
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
                                    if ui.button("New Folder").clicked() {
                                        let parent = self.workspace.selected_folder.clone();
                                        self.new_folder_in(&parent);
                                        ui.close_menu();
                                    }
                                    if ui.button("Refresh Files").clicked() {
                                        self.request("preview");
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
                            .selectable_label(
                                self.workspace.selected_folder.is_empty(),
                                RichText::new("Workspace").color(theme.ui.text.0).strong(),
                            )
                            .clicked()
                        {
                            self.workspace.selected_folder.clear();
                            self.explorer_root_open = true;
                            self.save_draft();
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if web_icon_button(ui, '\u{eac5}', "Collapse folders", &theme).clicked()
                            {
                                self.explorer_expanded.clear();
                            }
                            if web_icon_button(ui, '\u{eb37}', "Refresh workspace", &theme)
                                .clicked()
                            {
                                self.request("preview");
                            }
                            if web_icon_button(ui, '\u{ea7f}', "New document", &theme).clicked() {
                                self.new_file();
                            }
                            if web_icon_button(ui, '\u{ea83}', "New folder", &theme).clicked() {
                                let parent = self.workspace.selected_folder.clone();
                                self.new_folder_in(&parent);
                            }
                        });
                    });
                    if self.explorer_root_open {
                        ui.spacing_mut().item_spacing.y = 5.0;
                        self.workspace_tree(ui, "", &theme);
                        if self.workspace.folders.is_empty() && self.workspace.files.is_empty() {
                            ui.label(
                                RichText::new("No files yet. Add a file or folder.")
                                    .color(muted)
                                    .size(12.0),
                            );
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
        if self.tab == Tab::Document && self.preview_open {
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
        egui::TopBottomPanel::top("web-workspace-tabs")
            .exact_height(if narrow { 48.0 } else { 34.0 })
            .frame(
                egui::Frame::none()
                    .fill(panel)
                    .inner_margin(egui::Margin::symmetric(2.0, 0.0)),
            )
            .show(context, |ui| {
                let mut selected_document = None;
                let mut closed_document = None;
                let reveal_tab = self.reveal_tab;
                let row = ui.available_rect_before_wrap();
                let action_width = if self.tab == Tab::Document {
                    190.0_f32.min((row.width() - 80.0).max(0.0))
                } else {
                    0.0
                };
                let tabs_rect = egui::Rect::from_min_max(
                    row.min,
                    egui::pos2(row.max.x - action_width, row.max.y),
                );
                ui.allocate_ui_at_rect(tabs_rect, |ui| {
                    egui::ScrollArea::horizontal()
                        .id_source("web-open-tabs-scroll")
                        .animated(false)
                        .stick_to_right(true)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 1.0;
                                for (index, document) in self.documents.iter().enumerate() {
                                    let active =
                                        self.tab == Tab::Document && index == self.active_document;
                                    let name = if index == self.active_document {
                                        &self.source_name
                                    } else {
                                        &document.name
                                    };
                                    let dirty = if index == self.active_document {
                                        self.dirty
                                    } else {
                                        document.dirty
                                    };
                                    let tab_response = egui::Frame::none()
                                        .fill(if active { dark } else { panel })
                                        .inner_margin(egui::Margin::symmetric(10.0, 3.0))
                                        .show(ui, |ui| {
                                            ui.horizontal_centered(|ui| {
                                                ui.spacing_mut().item_spacing.x = 6.0;
                                                ui.set_min_height(if narrow { 41.0 } else { 27.0 });
                                                ui.label(
                                                    RichText::new("SC")
                                                        .strong()
                                                        .size(10.0)
                                                        .color(theme.ui.accent.0),
                                                );
                                                if ui
                                                    .add(
                                                        egui::Button::new(format!(
                                                            "{}{}",
                                                            name,
                                                            if dirty { "  •" } else { "" }
                                                        ))
                                                        .frame(false),
                                                    )
                                                    .clicked()
                                                {
                                                    selected_document = Some(index);
                                                }
                                                if ui
                                                    .add(
                                                        egui::Button::new("×")
                                                            .frame(false)
                                                            .min_size(egui::vec2(
                                                                if narrow { 32.0 } else { 18.0 },
                                                                if narrow { 40.0 } else { 22.0 },
                                                            )),
                                                    )
                                                    .on_hover_text("Close file")
                                                    .clicked()
                                                {
                                                    closed_document = Some(index);
                                                }
                                            });
                                        });
                                    if active && reveal_tab {
                                        ui.scroll_to_rect(
                                            tab_response.response.rect,
                                            Some(egui::Align::Center),
                                        );
                                    }
                                    if active {
                                        let rect = tab_response.response.rect;
                                        ui.painter().rect_filled(
                                            egui::Rect::from_min_max(
                                                egui::pos2(rect.left(), rect.bottom() - 2.0),
                                                rect.right_bottom(),
                                            ),
                                            0.0,
                                            theme.ui.accent.0,
                                        );
                                    }
                                }
                                for (tab, label) in [(Tab::Repl, "REPL"), (Tab::Plot, "Graphing")] {
                                    let tab_response = egui::Frame::none()
                                        .fill(if self.tab == tab { dark } else { panel })
                                        .inner_margin(egui::Margin::symmetric(12.0, 3.0))
                                        .show(ui, |ui| {
                                            ui.set_min_height(if narrow { 41.0 } else { 27.0 });
                                            if ui
                                                .add(egui::Button::new(label).frame(false))
                                                .clicked()
                                            {
                                                self.tab = tab;
                                            }
                                        });
                                    if self.tab == tab && reveal_tab {
                                        ui.scroll_to_rect(
                                            tab_response.response.rect,
                                            Some(egui::Align::Center),
                                        );
                                    }
                                }
                            });
                        });
                });
                if self.tab == Tab::Document {
                    let actions_rect =
                        egui::Rect::from_min_max(egui::pos2(tabs_rect.max.x, row.min.y), row.max);
                    ui.allocate_ui_at_rect(actions_rect, |ui| {
                        self.show_document_toolbar(ui, &theme);
                    });
                }
                self.reveal_tab = false;
                if let Some(index) = selected_document {
                    self.select_document(index);
                }
                if let Some(index) = closed_document {
                    self.close_document(index);
                }
            });

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
                            if ui
                                .selectable_label(
                                    self.bottom_tab == BottomTab::References,
                                    "REFERENCES",
                                )
                                .clicked()
                            {
                                self.bottom_tab = BottomTab::References;
                                if let Some(name) = self.symbol_at_cursor() {
                                    self.references_target = Some(name);
                                }
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
                        let panel_height = ui.available_height();
                        egui::ScrollArea::both()
                            .id_source(("web-output-scroll", self.bottom_tab as u8))
                            .max_height(panel_height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| match self.bottom_tab {
                            BottomTab::Output => {
                                if !self.output.is_empty() {
                                    ui.add(egui::Label::new(RichText::new(&self.output).monospace()).selectable(true).extend());
                                } else {
                                    ui.label(RichText::new("No output yet.").color(muted));
                                }
                                if let Some(error) = &self.output_error {
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
                            BottomTab::References => {
                                let mut query = self.references_target.clone().unwrap_or_default();
                                ui.horizontal(|ui| {
                                    ui.label("Symbol");
                                    if ui.add(TextEdit::singleline(&mut query).desired_width(180.0).hint_text("e.g. f or total")).changed() {
                                        self.references_target = (!query.trim().is_empty()).then(|| query.trim().to_owned());
                                    }
                                    if ui.button("Use editor cursor").clicked() {
                                        self.references_target = self.symbol_at_cursor();
                                    }
                                });
                                let Some(target) = self.references_target.as_deref() else {
                                    ui.label(
                                        RichText::new("Enter a symbol above, or use Shift+F12 in the editor.").color(muted),
                                    );
                                    return;
                                };
                                let index = SourceIndex::new(&self.source);
                                let unique = index.definition(target).is_some();
                                let references: Vec<_> = if unique { index.references(target) } else {
                                    index.occurrences.iter().filter(|item| item.name == target).collect()
                                };
                                ui.label(RichText::new(format!("{} {} for {target}", references.len(), if unique { "references" } else { "name matches (across scopes)" })).color(muted));
                                if references.is_empty() {
                                    ui.label(
                                        RichText::new("No occurrences found in this document. Check the symbol name.")
                                            .color(muted),
                                    );
                                }
                                for occurrence in references {
                                    let role = if occurrence.is_definition {
                                        "definition"
                                    } else {
                                        "reference"
                                    };
                                    let label = format!(
                                        "{}:{}:{}  {}  {}    {}",
                                        self.source_name,
                                        occurrence.line,
                                        occurrence.column,
                                        target,
                                        role,
                                        self.source.lines().nth(occurrence.line.saturating_sub(1)).unwrap_or("").trim()
                                    );
                                    if ui.selectable_label(false, label).clicked() {
                                        jump_to = Some((occurrence.line, occurrence.column));
                                    }
                                }
                            }
                        });
                    });
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(dark))
                .show(context, |ui| {
                    let completions = if editor_focused {
                        editor_assist::completions(&self.source, self.cursor_char_index)
                    } else {
                        editor_assist::Matches::empty(self.cursor_char_index)
                    };
                    let (user_completion_start, user_completions) =
                        if editor_focused && completions.len == 0 {
                            editor_assist::user_function_completions(
                                &self.source,
                                self.cursor_char_index,
                            )
                        } else {
                            (self.cursor_char_index, Vec::new())
                        };
                    let accept_completion = editor_focused
                        && (completions.len > 0 || !user_completions.is_empty())
                        && context.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                        });
                    if let Some(completion) =
                        accept_completion.then(|| completions.first()).flatten()
                    {
                        self.insert_editor_template(
                            editor_id,
                            completions.start_char,
                            self.cursor_char_index,
                            &completion.insertion.text(),
                        );
                    } else if accept_completion {
                        if let Some((name, _)) = user_completions.first() {
                            self.insert_editor_template(
                                editor_id,
                                user_completion_start,
                                self.cursor_char_index,
                                &format!("{name}($0)"),
                            );
                        }
                    }
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
                    )
                    .min((ui.available_width() - 96.0).max(0.0));
                    let output = egui::ScrollArea::both()
                        .show(ui, |ui| {
                            let output = TextEdit::multiline(&mut self.source)
                                .id(editor_id)
                                .font(editor_font.clone())
                                .code_editor()
                                .lock_focus(true)
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
                            if self.reveal_cursor {
                                if let Some(cursor) = output.cursor_range {
                                    let rect = output
                                        .galley
                                        .pos_from_cursor(&cursor.primary)
                                        .translate(output.galley_pos.to_vec2());
                                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                                }
                                self.reveal_cursor = false;
                            }
                            if self.line_numbers != LineNumberMode::Off && gutter >= 24.0 {
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
                            output
                        })
                        .inner;
                    let response = output.response.clone();
                    let cursor_range = output.cursor_range;
                    if let Some(cursor) = cursor_range {
                        self.cursor_char_index = cursor.primary.ccursor.index;
                        (self.cursor_line, self.cursor_column) =
                            line_column_at_char(&self.source, self.cursor_char_index);
                    }
                    let popup_completions = if response.has_focus() {
                        editor_assist::completions(&self.source, self.cursor_char_index)
                    } else {
                        editor_assist::Matches::empty(self.cursor_char_index)
                    };
                    let (popup_user_start, popup_user_completions) =
                        if response.has_focus() && popup_completions.len == 0 {
                            editor_assist::user_function_completions(
                                &self.source,
                                self.cursor_char_index,
                            )
                        } else {
                            (self.cursor_char_index, Vec::new())
                        };
                    let mut chosen_completion = None;
                    let mut chosen_user_completion = None;
                    if response.has_focus()
                        && (popup_completions.len > 0 || !popup_user_completions.is_empty())
                    {
                        if let Some(cursor) = cursor_range {
                            let caret = output.galley.pos_from_cursor(&cursor.primary);
                            let anchor =
                                output.galley_pos + egui::vec2(caret.left(), caret.bottom() + 4.0);
                            egui::Area::new(egui::Id::new("web-editor-completion"))
                                .order(egui::Order::Foreground)
                                .fixed_pos(anchor)
                                .constrain(true)
                                .show(context, |ui| {
                                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                                        ui.set_min_width(290.0);
                                        for (index, assist) in popup_completions.iter().enumerate()
                                        {
                                            let label = if index == 0 {
                                                format!("{}   Tab", assist.label)
                                            } else {
                                                assist.label.to_owned()
                                            };
                                            if ui.selectable_label(false, label).clicked() {
                                                chosen_completion = Some(assist);
                                            }
                                            ui.label(
                                                RichText::new(assist.detail)
                                                    .size(11.0)
                                                    .color(muted),
                                            );
                                        }
                                        for (index, (name, detail)) in
                                            popup_user_completions.iter().enumerate()
                                        {
                                            let label = if index == 0 {
                                                format!("{name}   Tab")
                                            } else {
                                                name.clone()
                                            };
                                            if ui.selectable_label(false, label).clicked() {
                                                chosen_user_completion = Some(name.clone());
                                            }
                                            ui.label(RichText::new(detail).size(11.0).color(muted));
                                        }
                                    });
                                });
                        }
                    } else if response.has_focus() {
                        let byte = char_to_byte(&self.source, self.cursor_char_index);
                        if let Some(call) =
                            SourceIndex::new(&self.source).call_at(&self.source, byte)
                        {
                            if !call.parameters.is_empty() {
                                if let Some(cursor) = cursor_range {
                                    let caret = output.galley.pos_from_cursor(&cursor.primary);
                                    let anchor = output.galley_pos
                                        + egui::vec2(caret.left(), caret.bottom() + 4.0);
                                    egui::Area::new(egui::Id::new("web-editor-signature"))
                                        .order(egui::Order::Foreground)
                                        .fixed_pos(anchor)
                                        .constrain(true)
                                        .show(context, |ui| {
                                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.monospace(format!("{}(", call.name));
                                                    for (index, parameter) in
                                                        call.parameters.iter().enumerate()
                                                    {
                                                        if index > 0 {
                                                            ui.monospace(", ");
                                                        }
                                                        let text =
                                                            RichText::new(parameter).monospace();
                                                        ui.label(
                                                            if index == call.active_parameter {
                                                                text.strong()
                                                                    .color(theme.ui.accent.0)
                                                            } else {
                                                                text
                                                            },
                                                        );
                                                    }
                                                    ui.monospace(")");
                                                });
                                            });
                                        });
                                }
                            }
                        }
                    }
                    if let Some(completion) = chosen_completion {
                        self.insert_editor_template(
                            editor_id,
                            popup_completions.start_char,
                            self.cursor_char_index,
                            &completion.insertion.text(),
                        );
                    }
                    if let Some(name) = chosen_user_completion {
                        self.insert_editor_template(
                            editor_id,
                            popup_user_start,
                            self.cursor_char_index,
                            &format!("{name}($0)"),
                        );
                    }
                    if let Some(pointer) = response.hover_pos() {
                        let cursor = output.galley.cursor_from_pos(pointer - output.galley_pos);
                        let byte = char_to_byte(&self.source, cursor.ccursor.index);
                        if self.hover_key != Some(byte) {
                            self.hover_key = Some(byte);
                            self.hover_since = now;
                            self.hover_value = None;
                        } else if now - self.hover_since >= 0.45 {
                            if self.hover_value.is_none() {
                                self.request_hover(byte);
                            }
                            if let Some(value) = &self.hover_value {
                                if !value.is_empty() {
                                    response.clone().on_hover_text(format!("= {value}"));
                                }
                            }
                        } else {
                            context.request_repaint_after(Duration::from_millis(460));
                        }
                    }
                    if response.changed() {
                        self.hover_key = None;
                        self.hover_generation = 0;
                        self.hover_value = None;
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
                self.jump_to_source(line, column);
            }
        } else if self.tab == Tab::Repl {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(dark))
                .show(context, |ui| self.show_repl(ui, &theme));
        } else {
            egui::TopBottomPanel::top("plot-controls")
                .frame(
                    egui::Frame::none()
                        .fill(panel)
                        .inner_margin(egui::Margin::symmetric(16.0, 12.0)),
                )
                .show(context, |ui| {
                    ui.label(
                        RichText::new("Graphing")
                            .color(theme.ui.text.0)
                            .strong()
                            .size(16.0),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("y =").monospace().size(16.0));
                        let expression = ui.add(
                            TextEdit::singleline(&mut self.plot_expression)
                                .hint_text("sin(x) from -10 to 10 y from -2 to 2")
                                .font(FontId::monospace(16.0))
                                .desired_width((ui.available_width() - 76.0).max(60.0)),
                        );
                        if expression.changed() && self.auto_greek {
                            expand_greek_shortcuts(&mut self.plot_expression);
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Plot").color(theme.contrast_on_accent()),
                                )
                                .fill(theme.ui.accent.0)
                                .min_size(egui::vec2(64.0, 28.0)),
                            )
                            .clicked()
                            || (expression.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                        {
                            self.plot_custom_view = false;
                            self.plot_fit_y_pending = false;
                            self.plot_due = None;
                            self.request("plot");
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
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
                    if let Some(error) = &self.plot_error {
                        ui.colored_label(Color32::from_rgb(249, 119, 127), error);
                    }
                    if !self.plot_series.is_empty() {
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            for (index, series) in self.plot_series.iter().enumerate() {
                                let color = graph_colors(&theme)[index % PLOT_COLORS.len()];
                                let (swatch, _) = ui.allocate_exact_size(
                                    egui::vec2(18.0, 16.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().line_segment(
                                    [swatch.left_center(), swatch.right_center()],
                                    egui::Stroke::new(2.5, color),
                                );
                                ui.label(RichText::new(&series.label).monospace().size(12.0));
                                ui.add_space(12.0);
                            }
                        });
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
                self.error.as_deref().unwrap_or(""),
                self.pending,
                0.0,
                44.0,
                screen.width(),
                screen.height() - 70.0,
                context.pixels_per_point(),
                false,
            );
        }
        if !preview_visible {
            hide_preview();
        }
        if self.tab != Tab::Repl
            || self.settings_open
            || self.quick_open
            || context.memory(|memory| memory.any_popup_open())
        {
            hide_repl();
        }
        self.show_settings(context);
        self.show_save_as(context);
        self.show_close_document(context);
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
                        self.load_source_named(
                            "mit-18-06-session-22.scas".to_owned(),
                            MIT_EXAMPLE.to_owned(),
                        );
                    }
                    1 => {
                        self.load_source_named("starter.scas".to_owned(), STARTER.to_owned());
                    }
                    2 => self.tab = Tab::Plot,
                    _ => self.tab = Tab::Repl,
                }
                self.quick_open = false;
                self.quick_open_query.clear();
            }
        }
        if self.tab != prior_tab {
            self.reveal_tab = true;
        }
    }
}
