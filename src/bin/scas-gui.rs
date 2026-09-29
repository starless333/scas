#[path = "scas_gui/editor_assist.rs"]
mod editor_assist;
#[path = "scas_gui/math_preview.rs"]
mod math_preview;
#[path = "scas_gui/theme.rs"]
mod theme;

use eframe::egui::{
    self, text::LayoutJob, FontData, FontDefinitions, FontFamily, FontId, RichText, TextEdit,
    TextFormat, TextStyle,
};
use scas::ast::statement::FunctionBody;
use scas::ast::StatementKind;
use scas::document::Document;
use scas::execute;
use scas::interpreter::Interpreter;
use scas::latex::LatexRenderer;
use scas::lexer::{Scanner, SymbolTable, TokenType};
use scas::parser::Parser;
use scas::Limits;
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use theme::Theme;

const STARTER: &str = "/// A live, typeset result\n\
let angle = π / 6;\n\
\\[ sin(angle) => \\]\n\
print(sin(angle));\n";
const DEFAULT_AUTO_GREEK: bool = true;

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

#[derive(Clone)]
struct TreeEntry {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
}

struct RunResult {
    output: String,
    failed: bool,
}

struct LatexRender {
    rgba: Vec<u8>,
    size: [usize; 2],
}

struct HighlightCache {
    source: String,
    theme: usize,
    font_size_bits: u32,
    monospace: bool,
    diagnostics_revision: u64,
    job: LayoutJob,
}

struct AnalysisResult {
    generation: u64,
    document: Result<Document, String>,
    diagnostics: Vec<EditorDiagnostic>,
}

#[derive(Clone, Debug)]
struct EditorDiagnostic {
    byte_start: usize,
    byte_end: usize,
    line: usize,
    column: usize,
    message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineNumberMode {
    Off,
    Absolute,
    Relative,
    Hybrid,
}

#[derive(Clone, Copy)]
enum ActivityIcon {
    Explorer,
    Preview,
    Problems,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceTab {
    Document,
    Repl,
    Graphing,
}

struct ReplEntry {
    source: String,
    input_latex: Option<String>,
    result_latex: Option<String>,
    printed: String,
    error: Option<String>,
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
}

struct ScasApp {
    source: String,
    output: String,
    current_file: Option<PathBuf>,
    project_root: PathBuf,
    files: Vec<TreeEntry>,
    collapsed_dirs: HashSet<PathBuf>,
    dirty: bool,
    auto_greek: bool,
    run_result: Option<Receiver<RunResult>>,
    status: String,
    themes: Vec<Theme>,
    selected_theme: usize,
    editor_font_size: f32,
    editor_monospace: bool,
    settings_open: bool,
    theme_path: String,
    font_path: String,
    preview: String,
    preview_document: Option<Document>,
    preview_error: Option<String>,
    preview_open: bool,
    files_open: bool,
    diagnostics_open: bool,
    diagnostics: Vec<EditorDiagnostic>,
    analysis_due: Option<Instant>,
    analysis_result: Option<Receiver<AnalysisResult>>,
    analysis_generation: u64,
    buffer_generation: u64,
    latex_render: Option<Receiver<(u64, Result<LatexRender, String>)>>,
    latex_pending: Option<(String, [u8; 3], u64)>,
    last_latex_key: Option<u64>,
    system_latex_preview: bool,
    latex_texture: Option<egui::TextureHandle>,
    latex_render_error: Option<String>,
    line_numbers: LineNumberMode,
    current_line: usize,
    cursor_char_index: usize,
    diagnostics_revision: u64,
    highlight_cache: Option<HighlightCache>,
    workspace_tab: WorkspaceTab,
    repl_input: String,
    repl_input_latex: Option<String>,
    repl_entries: Vec<ReplEntry>,
    repl_symbols: SymbolTable,
    repl_interpreter: Interpreter<Vec<u8>>,
}

impl ScasApp {
    fn new(context: &eframe::CreationContext<'_>) -> Self {
        let themes = Theme::bundled_themes();
        themes[0].apply(&context.egui_ctx);
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let files = collect_tree(&project_root);
        let repl_symbols = SymbolTable::new();
        let repl_interpreter = Interpreter::new(repl_symbols.clone(), Vec::new());
        let mut app = Self {
            source: STARTER.to_owned(),
            output: "Run the program to see its output.".to_owned(),
            current_file: None,
            project_root,
            files,
            collapsed_dirs: HashSet::new(),
            dirty: false,
            auto_greek: DEFAULT_AUTO_GREEK,
            run_result: None,
            status: "Ready".to_owned(),
            themes,
            selected_theme: 0,
            editor_font_size: 15.0,
            editor_monospace: true,
            settings_open: false,
            theme_path: "themes/dark-modern.toml".to_owned(),
            font_path: String::new(),
            preview: String::new(),
            preview_document: None,
            preview_error: None,
            preview_open: true,
            files_open: true,
            diagnostics_open: false,
            diagnostics: Vec::new(),
            analysis_due: None,
            analysis_result: None,
            analysis_generation: 0,
            buffer_generation: 0,
            latex_render: None,
            latex_pending: None,
            last_latex_key: None,
            system_latex_preview: true,
            latex_texture: None,
            latex_render_error: None,
            line_numbers: LineNumberMode::Absolute,
            current_line: 0,
            cursor_char_index: 0,
            diagnostics_revision: 0,
            highlight_cache: None,
            workspace_tab: WorkspaceTab::Document,
            repl_input: String::new(),
            repl_input_latex: None,
            repl_entries: Vec::new(),
            repl_symbols,
            repl_interpreter,
        };
        app.start_analysis();
        app
    }

    fn reset_repl(&mut self) {
        self.repl_symbols = SymbolTable::new();
        self.repl_interpreter = Interpreter::new(self.repl_symbols.clone(), Vec::new());
        self.repl_entries.clear();
        self.repl_input.clear();
        self.repl_input_latex = None;
        self.set_status("REPL session reset");
    }

    fn refresh_repl_latex(&mut self) {
        self.repl_input_latex = repl_expression_latex(&self.repl_input, &self.repl_symbols);
    }

    fn submit_repl(&mut self) {
        let source = self.repl_input.trim().to_owned();
        if source.is_empty() {
            return;
        }
        if source == ":clear" {
            self.repl_entries.clear();
            self.repl_input.clear();
            self.repl_input_latex = None;
            self.set_status("REPL history cleared");
            return;
        }
        if source == ":reset" {
            self.reset_repl();
            return;
        }

        let submission = terminate_repl_source(&source);
        let mut scanner = Scanner::with_symbols(&submission, self.repl_symbols.clone());
        let tokens = scanner.scan_tokens().clone();
        let scan_error = scanner
            .errors()
            .iter()
            .map(|error| error.render(&submission, "<repl>"))
            .collect::<Vec<_>>()
            .join("\n");
        self.repl_symbols = scanner.into_symbols();
        self.repl_interpreter.set_symbols(self.repl_symbols.clone());

        let input_latex = self.repl_input_latex.clone();
        let mut result_latex = None;
        let mut error = (!scan_error.is_empty()).then_some(scan_error);
        if error.is_none() {
            match Parser::new(tokens).parse() {
                Ok(program) => match self.repl_interpreter.run_interactive_value(&program) {
                    Ok(Some(value)) => {
                        result_latex =
                            Some(LatexRenderer::new(self.repl_interpreter.symbols()).value(&value));
                    }
                    Ok(None) => {}
                    Err(runtime) => error = Some(runtime.render(&submission, "<repl>")),
                },
                Err(parse) => error = Some(parse.render(&submission, "<repl>")),
            }
        }
        let printed = String::from_utf8_lossy(&std::mem::take(self.repl_interpreter.output_mut()))
            .into_owned();
        let succeeded = error.is_none();
        self.repl_entries.push(ReplEntry {
            source,
            input_latex,
            result_latex,
            printed,
            error,
        });
        self.repl_input.clear();
        self.repl_input_latex = None;
        self.set_status(if succeeded {
            "REPL expression evaluated"
        } else {
            "REPL expression needs attention"
        });
    }

    fn run(&mut self) {
        if self.run_result.is_some() {
            return;
        }
        let source = self.source.clone();
        let source_name = self
            .current_file
            .as_deref()
            .and_then(Path::to_str)
            .unwrap_or("<editor>")
            .to_owned();
        let (sender, receiver) = mpsc::channel();
        self.run_result = Some(receiver);
        self.set_status("Running…");
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let error = execute(&source, &mut bytes).err();
            let mut output = String::from_utf8_lossy(&bytes).into_owned();
            if let Some(error) = error {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&error.render(&source, &source_name));
                let _ = sender.send(RunResult {
                    output,
                    failed: true,
                });
            } else {
                if output.is_empty() {
                    output.push_str("Program completed without output.");
                }
                let _ = sender.send(RunResult {
                    output,
                    failed: false,
                });
            }
        });
    }

    fn poll_run(&mut self, context: &egui::Context) {
        let Some(receiver) = &self.run_result else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.output = result.output;
                self.set_status(if result.failed {
                    "Run failed"
                } else {
                    "Run finished"
                });
                self.run_result = None;
            }
            Err(TryRecvError::Empty) => context.request_repaint_after(Duration::from_millis(50)),
            Err(TryRecvError::Disconnected) => {
                self.output.clear();
                self.output
                    .push_str("The execution worker stopped unexpectedly.");
                self.set_status("Run failed");
                self.run_result = None;
            }
        }
    }

    fn open(&mut self, path: PathBuf) {
        match fs::read_to_string(&path) {
            Ok(source) => {
                self.source = source;
                self.current_file = Some(path.clone());
                self.dirty = false;
                self.cursor_char_index = 0;
                self.buffer_generation = self.buffer_generation.wrapping_add(1);
                self.analysis_generation = self.analysis_generation.wrapping_add(1);
                self.analysis_due = None;
                self.status = format!("Opened {}", path.display());
                self.start_analysis();
            }
            Err(error) => self.status = format!("Could not open {}: {error}", path.display()),
        }
    }

    fn save(&mut self) {
        let Some(path) = self.current_file.as_ref() else {
            self.set_status("Choose a .scas file in the tree before saving");
            return;
        };
        match fs::write(path, &self.source) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", path.display());
            }
            Err(error) => self.status = format!("Could not save {}: {error}", path.display()),
        }
    }

    fn export_stem(&self) -> PathBuf {
        self.current_file
            .clone()
            .unwrap_or_else(|| self.project_root.join("document.scas"))
    }

    fn export_tex(&mut self) -> Option<PathBuf> {
        let document = match scas::document::build(&self.source) {
            Ok(document) => document,
            Err(error) => {
                self.status = format!("Cannot export: {error}");
                return None;
            }
        };
        let path = self.export_stem().with_extension("tex");
        match fs::write(&path, document.latex()) {
            Ok(()) => {
                self.status = format!("Exported {}", path.display());
                Some(path)
            }
            Err(error) => {
                self.status = format!("Could not export {}: {error}", path.display());
                None
            }
        }
    }

    fn export_pdf(&mut self) {
        let Some(tex_path) = self.export_tex() else {
            return;
        };
        let Some(directory) = tex_path.parent() else {
            self.set_status("Could not determine the export directory");
            return;
        };
        let Some(file_name) = tex_path.file_name() else {
            self.set_status("Could not determine the TeX filename");
            return;
        };
        match Command::new("pdflatex")
            .arg("-interaction=nonstopmode")
            .arg("-halt-on-error")
            .arg(file_name)
            .current_dir(directory)
            .output()
        {
            Ok(output) if output.status.success() => {
                self.status = format!("Exported {}", tex_path.with_extension("pdf").display());
            }
            Ok(output) => {
                let message = String::from_utf8_lossy(&output.stdout);
                let detail = message
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("pdflatex failed");
                self.status = format!("PDF export failed: {detail}");
            }
            Err(error) => {
                self.status = format!("PDF export requires pdflatex: {error}");
            }
        }
    }

    fn new_file(&mut self) {
        self.source.clear();
        self.current_file = None;
        self.dirty = false;
        self.cursor_char_index = 0;
        self.buffer_generation = self.buffer_generation.wrapping_add(1);
        self.analysis_generation = self.analysis_generation.wrapping_add(1);
        self.analysis_due = None;
        self.set_status("New buffer");
        self.start_analysis();
    }

    fn set_status(&mut self, status: &str) {
        self.status.clear();
        self.status.push_str(status);
    }

    fn insert_editor_template(
        &mut self,
        context: &egui::Context,
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
            |index| {
                let mut text = String::with_capacity(template.len().saturating_sub(2));
                text.push_str(&template[..index]);
                text.push_str(&template[index + 2..]);
                text
            },
        );
        let start_byte = char_to_byte(&self.source, start_char);
        let end_byte = char_to_byte(&self.source, end_char);
        let before = self.source.clone();
        self.source.replace_range(start_byte..end_byte, &insertion);
        self.cursor_char_index = start_char + cursor_offset;

        let mut state = TextEdit::load_state(context, editor_id).unwrap_or_default();
        let old_range = state
            .cursor
            .char_range()
            .unwrap_or_else(|| egui::text::CCursorRange::one(egui::text::CCursor::new(end_char)));
        let mut undoer = state.undoer();
        undoer.add_undo(&(old_range, before));
        state.set_undoer(undoer);
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(self.cursor_char_index),
            )));
        TextEdit::store_state(context, editor_id, state);
        context.memory_mut(|memory| memory.request_focus(editor_id));
        self.highlight_cache = None;
        self.dirty = true;
        self.schedule_analysis(context);
    }

    fn start_analysis(&mut self) {
        if self.analysis_result.is_some() {
            return;
        }
        let source = self.source.clone();
        let generation = self.analysis_generation;
        let (sender, receiver) = mpsc::channel();
        self.analysis_result = Some(receiver);
        std::thread::spawn(move || {
            let diagnostics = source_diagnostics(&source);
            let document = scas::document::build_with_limits(&source, live_preview_limits())
                .map_err(|error| error.to_string());
            let _ = sender.send(AnalysisResult {
                generation,
                document,
                diagnostics,
            });
        });
    }

    fn apply_analysis(&mut self, result: AnalysisResult) {
        if result.generation != self.analysis_generation {
            return;
        }
        let mut diagnostics = result.diagnostics;
        match result.document {
            Ok(document) => {
                self.preview = document.latex();
                let text_color = self.themes[self.selected_theme].ui.text.0;
                if self.preview_open && self.system_latex_preview {
                    self.start_latex_render(
                        document.body(),
                        [text_color.r(), text_color.g(), text_color.b()],
                    );
                }
                for diagnostic in &document.diagnostics {
                    let byte_start = diagnostic.span.byte_start.min(self.source.len());
                    let byte_end = diagnostic
                        .span
                        .byte_end
                        .max(byte_start.saturating_add(1))
                        .min(self.source.len());
                    diagnostics.push(EditorDiagnostic {
                        byte_start,
                        byte_end,
                        line: diagnostic.span.line,
                        column: diagnostic.span.column,
                        message: diagnostic.message.clone(),
                    });
                }
                self.diagnostics = diagnostics;
                self.diagnostics_revision = self.diagnostics_revision.wrapping_add(1);
                self.highlight_cache = None;
                self.preview_document = Some(document);
                self.preview_error = None;
            }
            Err(error) => {
                self.diagnostics = diagnostics;
                self.diagnostics_revision = self.diagnostics_revision.wrapping_add(1);
                self.highlight_cache = None;
                // Keep the last successful document visible while the user is
                // in the middle of typing an incomplete expression.
                self.preview_error = Some(error.to_string());
            }
        }
    }

    fn schedule_analysis(&mut self, context: &egui::Context) {
        const DEBOUNCE: Duration = Duration::from_millis(900);
        self.analysis_generation = self.analysis_generation.wrapping_add(1);
        self.analysis_due = Some(Instant::now() + DEBOUNCE);
        // Do not show stale errors while the user is repairing the source.
        self.diagnostics.clear();
        self.diagnostics_revision = self.diagnostics_revision.wrapping_add(1);
        self.highlight_cache = None;
        self.preview_error = None;
        context.request_repaint_after(DEBOUNCE);
    }

    fn poll_analysis(&mut self, context: &egui::Context) {
        if let Some(receiver) = &self.analysis_result {
            match receiver.try_recv() {
                Ok(result) => {
                    self.analysis_result = None;
                    self.apply_analysis(result);
                }
                Err(TryRecvError::Empty) => {
                    context.request_repaint_after(Duration::from_millis(100));
                }
                Err(TryRecvError::Disconnected) => {
                    self.analysis_result = None;
                    self.preview_error = Some("analysis worker stopped unexpectedly".to_owned());
                }
            }
        }

        if let Some(due) = self.analysis_due {
            let now = Instant::now();
            if now >= due && self.analysis_result.is_none() {
                self.analysis_due = None;
                self.start_analysis();
            } else if now < due {
                context.request_repaint_after(due.duration_since(now));
            }
        }
    }

    fn start_latex_render(&mut self, body: String, color: [u8; 3]) {
        if !self.system_latex_preview || !self.preview_open {
            return;
        }
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);
        color.hash(&mut hasher);
        let key = hasher.finish();
        if self.last_latex_key == Some(key)
            || self
                .latex_pending
                .as_ref()
                .is_some_and(|(_, _, pending_key)| *pending_key == key)
        {
            return;
        }
        if self.latex_render.is_some() {
            self.latex_pending = Some((body, color, key));
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.latex_render = Some(receiver);
        self.latex_render_error = None;
        std::thread::spawn(move || {
            let result = render_latex_png(&body, color);
            let _ = sender.send((key, result));
        });
    }

    fn poll_latex_render(&mut self, context: &egui::Context) {
        let Some(receiver) = &self.latex_render else {
            return;
        };
        let event = receiver.try_recv();
        match event {
            Ok((key, Ok(render))) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(render.size, &render.rgba);
                self.latex_texture = Some(context.load_texture(
                    "latex-preview",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
                self.last_latex_key = Some(key);
                self.latex_render = None;
            }
            Ok((_key, Err(error))) => {
                self.latex_render_error = Some(error);
                self.latex_render = None;
            }
            Err(TryRecvError::Empty) => context.request_repaint_after(Duration::from_millis(100)),
            Err(TryRecvError::Disconnected) => {
                self.latex_render_error = Some("LaTeX preview worker stopped".to_owned());
                self.latex_render = None;
            }
        }
        if self.latex_render.is_none() {
            if let Some((body, color, _)) = self.latex_pending.take() {
                self.start_latex_render(body, color);
            }
        }
    }

    fn title(&self) -> String {
        let name = self
            .current_file
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled");
        format!("{name}{}", if self.dirty { " •" } else { "" })
    }

    fn load_theme(&mut self, context: &egui::Context) {
        let path = PathBuf::from(self.theme_path.trim());
        match Theme::load(&path) {
            Ok(theme) => {
                self.status = format!("Loaded theme {}", theme.name);
                self.themes.push(theme);
                self.selected_theme = self.themes.len() - 1;
                self.themes[self.selected_theme].apply(context);
                if let Some(document) = &self.preview_document {
                    let color = self.themes[self.selected_theme].ui.text.0;
                    self.start_latex_render(document.body(), [color.r(), color.g(), color.b()]);
                }
            }
            Err(error) => self.status = error,
        }
    }

    fn load_font(&mut self, context: &egui::Context) {
        let path = PathBuf::from(self.font_path.trim());
        match fs::read(&path) {
            Ok(bytes) => {
                let name = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("custom-editor-font")
                    .to_owned();
                let mut fonts = FontDefinitions::default();
                fonts
                    .font_data
                    .insert(name.clone(), FontData::from_owned(bytes));
                fonts
                    .families
                    .entry(FontFamily::Monospace)
                    .or_default()
                    .insert(0, name.clone());
                fonts
                    .families
                    .entry(FontFamily::Proportional)
                    .or_default()
                    .insert(0, name.clone());
                context.set_fonts(fonts);
                self.status = format!("Loaded font {name}");
            }
            Err(error) => self.status = format!("Could not load {}: {error}", path.display()),
        }
    }

    fn show_repl(&mut self, ui: &mut egui::Ui, context: &egui::Context, theme: &Theme) {
        let enter_pressed = context.input(|input| {
            input.key_pressed(egui::Key::Enter)
                && !input.modifiers.shift
                && !input.modifiers.ctrl
                && !input.modifiers.command
        });
        let history_height = (ui.available_height() - 154.0).max(120.0);
        egui::ScrollArea::vertical()
            .id_source("gui-repl-history")
            .max_height(history_height)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if self.repl_entries.is_empty() {
                    ui.add_space(28.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("Start a calculation")
                                .strong()
                                .size(18.0)
                                .color(theme.ui.text.0),
                        );
                        ui.label(
                            RichText::new(
                                "Enter an expression below. Definitions stay available for the session.",
                            )
                            .size(12.0)
                            .color(theme.ui.muted.0),
                        );
                        ui.add_space(6.0);
                        ui.monospace("Try:  let A = [1, 1; 1, 0];   then   A^10");
                    });
                }
                for (index, entry) in self.repl_entries.iter().enumerate() {
                    egui::Frame::none()
                        .inner_margin(egui::Margin::symmetric(12.0, 12.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{}", index + 1))
                                        .monospace()
                                        .size(10.5)
                                        .color(theme.ui.muted.0),
                                );
                                ui.label(
                                    RichText::new(&entry.source)
                                        .monospace()
                                        .size(11.0)
                                        .color(theme.ui.muted.0),
                                );
                            });
                            ui.add_space(5.0);
                            ui.columns(2, |columns| {
                                if let Some(latex) = &entry.input_latex {
                                    math_preview::show_expression(
                                        &mut columns[0],
                                        latex,
                                        theme.ui.text.0,
                                        20.0,
                                    );
                                } else {
                                    columns[0].label(
                                        RichText::new(&entry.source)
                                            .monospace()
                                            .color(theme.ui.text.0),
                                    );
                                }
                                if let Some(latex) = &entry.result_latex {
                                    columns[1].with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            math_preview::show_expression(
                                                ui,
                                                latex,
                                                theme.ui.accent.0,
                                                20.0,
                                            );
                                        },
                                    );
                                }
                            });
                            if !entry.printed.trim().is_empty() {
                                ui.label(
                                    RichText::new(entry.printed.trim_end())
                                        .monospace()
                                        .size(12.0),
                                );
                            }
                            if let Some(error) = &entry.error {
                                ui.colored_label(theme.syntax.operator.0, error);
                            }
                        });
                    ui.separator();
                    ui.add_space(8.0);
                }
            });

        ui.add_space(8.0);
        let input_id = egui::Id::new("gui-repl-input");
        let mut submit = false;
        egui::Frame::none()
            .fill(theme.ui.panel.0)
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .stroke(egui::Stroke::new(
                1.0,
                theme.ui.accent.0.gamma_multiply(0.65),
            ))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("LIVE MATH ENTRY")
                            .strong()
                            .size(10.5)
                            .color(theme.ui.accent.0),
                    );
                    ui.label(
                        RichText::new("Enter to evaluate")
                            .size(10.5)
                            .color(theme.ui.muted.0),
                    );
                });
                ui.add_space(5.0);
                egui::Frame::none()
                    .fill(theme.ui.editor.0)
                    .inner_margin(egui::Margin::symmetric(10.0, 9.0))
                    .show(ui, |ui| {
                        ui.set_min_height(42.0);
                        if let Some(latex) = &self.repl_input_latex {
                            math_preview::show_expression(ui, latex, theme.ui.text.0, 23.0);
                        } else if self.repl_input.trim().is_empty() {
                            ui.label(
                                RichText::new("Your expression will typeset here as you work")
                                    .italics()
                                    .color(theme.ui.muted.0),
                            );
                        } else {
                            ui.label(
                                RichText::new("Keep typing to complete the expression…")
                                    .italics()
                                    .color(theme.ui.muted.0),
                            );
                        }
                    });
                ui.add_space(6.0);
                let response = ui.add(
                    TextEdit::singleline(&mut self.repl_input)
                        .id(input_id)
                        .font(TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .hint_text("Type SCAS math, for example integral(sin(x), x)"),
                );
                if response.changed() {
                    if self.auto_greek {
                        expand_greek_shortcuts(&mut self.repl_input);
                    }
                    self.refresh_repl_latex();
                    context.request_repaint();
                }
                submit = (response.has_focus() || response.lost_focus()) && enter_pressed;
                ui.horizontal(|ui| {
                    if ui.button("Evaluate").clicked() {
                        submit = true;
                    }
                    ui.label(
                        RichText::new("Commands: :clear · :reset")
                            .size(10.5)
                            .color(theme.ui.muted.0),
                    );
                });
            });
        if submit {
            self.submit_repl();
            context.memory_mut(|memory| memory.request_focus(input_id));
        }
    }

    fn show_graphing(&self, ui: &mut egui::Ui, theme: &Theme) {
        let rect = ui.available_rect_before_wrap();
        let center = rect.center();
        let axis = theme.ui.muted.0.gamma_multiply(0.18);
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), center.y),
                egui::pos2(rect.right(), center.y),
            ],
            egui::Stroke::new(1.0, axis),
        );
        ui.painter().line_segment(
            [
                egui::pos2(center.x, rect.top()),
                egui::pos2(center.x, rect.bottom()),
            ],
            egui::Stroke::new(1.0, axis),
        );
        ui.allocate_ui_at_rect(rect, |ui| {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Graphing workspace")
                            .strong()
                            .size(18.0)
                            .color(theme.ui.text.0),
                    );
                    ui.label(
                        RichText::new("Reserved for the graphing calculator")
                            .size(12.0)
                            .color(theme.ui.muted.0),
                    );
                });
            });
        });
    }

    fn settings(&mut self, context: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        let mut chosen_theme = None;
        let mut load_theme = false;
        let mut load_font = false;
        let previous_system_preview = self.system_latex_preview;
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
                    ui.text_edit_singleline(&mut self.theme_path);
                    load_theme = ui.button("Load").clicked();
                });
                ui.separator();
                ui.label("Line numbers");
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
                ui.separator();
                ui.label("Preview performance");
                ui.checkbox(&mut self.system_latex_preview, "Computer Modern preview");
                ui.small(
                    "Uses system LaTeX for faithful typesetting. Off uses the fast structured fallback.",
                );
                ui.separator();
                ui.label("Editor font");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.editor_monospace, true, "Monospace");
                    ui.radio_value(&mut self.editor_monospace, false, "Proportional");
                    ui.add(egui::Slider::new(&mut self.editor_font_size, 9.0..=32.0).suffix(" pt"));
                });
                ui.horizontal(|ui| {
                    ui.label("Font file");
                    ui.text_edit_singleline(&mut self.font_path);
                    load_font = ui.button("Load").clicked();
                });
                ui.small("Theme files use TOML; font files may be .ttf or .otf.");
            });
        self.settings_open = open;
        if !previous_system_preview && self.system_latex_preview {
            if let Some(document) = &self.preview_document {
                let body = document.body();
                let color = self.themes[self.selected_theme].ui.text.0;
                self.start_latex_render(body, [color.r(), color.g(), color.b()]);
            }
        } else if previous_system_preview && !self.system_latex_preview {
            self.latex_pending = None;
        }
        if let Some(index) = chosen_theme {
            self.themes[index].apply(context);
            self.status = format!("Using {}", self.themes[index].name);
            if let Some(document) = &self.preview_document {
                let color = self.themes[self.selected_theme].ui.text.0;
                self.start_latex_render(document.body(), [color.r(), color.g(), color.b()]);
            }
        }
        if load_theme {
            self.load_theme(context);
        }
        if load_font {
            self.load_font(context);
        }
    }
}

fn terminate_repl_source(source: &str) -> String {
    let trimmed = source.trim_end();
    if trimmed.ends_with(';') || trimmed.ends_with('}') {
        source.to_owned()
    } else {
        format!("{source};")
    }
}

fn repl_expression_latex(source: &str, symbols: &SymbolTable) -> Option<String> {
    let source = source.trim();
    if source.is_empty() || source.starts_with(':') {
        return None;
    }
    let submission = terminate_repl_source(source);
    let mut scanner = Scanner::with_symbols(&submission, symbols.clone());
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

impl eframe::App for ScasApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_run(context);
        self.poll_analysis(context);
        self.poll_latex_render(context);
        self.settings(context);

        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.save();
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)) {
            match self.workspace_tab {
                WorkspaceTab::Document => self.run(),
                WorkspaceTab::Repl => self.submit_repl(),
                WorkspaceTab::Graphing => {}
            }
        }

        let theme = self.themes[self.selected_theme].clone();
        let surface_title = match self.workspace_tab {
            WorkspaceTab::Document => self.title(),
            WorkspaceTab::Repl => "Interactive REPL".to_owned(),
            WorkspaceTab::Graphing => "Graphing calculator".to_owned(),
        };
        egui::TopBottomPanel::top("command-bar")
            .exact_height(36.0)
            .frame(
                egui::Frame::none()
                    .fill(theme.ui.panel.0)
                    .inner_margin(egui::Margin::symmetric(8.0, 5.0))
                    .stroke(egui::Stroke::new(
                        1.0,
                        theme.ui.muted.0.gamma_multiply(0.22),
                    )),
            )
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        RichText::new("SCAS")
                            .strong()
                            .size(13.0)
                            .color(theme.ui.text.0),
                    );
                    ui.menu_button("File", |ui| {
                        if ui.button("New File").clicked() {
                            self.new_file();
                            ui.close_menu();
                        }
                        if ui.button("Save").clicked() {
                            self.save();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Export TeX…").clicked() {
                            self.export_tex();
                            ui.close_menu();
                        }
                        if ui.button("Export PDF…").clicked() {
                            self.export_pdf();
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("View", |ui| {
                        if ui.checkbox(&mut self.files_open, "Explorer").clicked() {
                            ui.close_menu();
                        }
                        if ui
                            .checkbox(&mut self.preview_open, "Document Preview")
                            .clicked()
                        {
                            ui.close_menu();
                        }
                        if ui
                            .checkbox(&mut self.diagnostics_open, "Problems")
                            .clicked()
                        {
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Run", |ui| {
                        let label = match self.workspace_tab {
                            WorkspaceTab::Document => "Run Document",
                            WorkspaceTab::Repl => "Evaluate REPL Entry",
                            WorkspaceTab::Graphing => "Graphing not available yet",
                        };
                        if ui
                            .add_enabled(
                                self.workspace_tab != WorkspaceTab::Graphing
                                    && self.run_result.is_none(),
                                egui::Button::new(label),
                            )
                            .clicked()
                        {
                            match self.workspace_tab {
                                WorkspaceTab::Document => self.run(),
                                WorkspaceTab::Repl => self.submit_repl(),
                                WorkspaceTab::Graphing => {}
                            }
                            ui.close_menu();
                        }
                    });
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(format!("{surface_title} — SCAS"))
                            .size(12.0)
                            .color(theme.ui.muted.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let running = self.run_result.is_some();
                        let action_label = match self.workspace_tab {
                            WorkspaceTab::Document if running => "Running…",
                            WorkspaceTab::Document => "Run",
                            WorkspaceTab::Repl => "Evaluate",
                            WorkspaceTab::Graphing => "Reserved",
                        };
                        if ui
                            .add_enabled(
                                !running && self.workspace_tab != WorkspaceTab::Graphing,
                                egui::Button::new(action_label)
                                    .fill(theme.ui.accent.0)
                                    .stroke(egui::Stroke::NONE),
                            )
                            .on_hover_text("Run the active workspace · Ctrl+Enter")
                            .clicked()
                        {
                            match self.workspace_tab {
                                WorkspaceTab::Document => self.run(),
                                WorkspaceTab::Repl => self.submit_repl(),
                                WorkspaceTab::Graphing => {}
                            }
                        }
                        if ui
                            .small_button("Settings")
                            .on_hover_text("Workspace settings")
                            .clicked()
                        {
                            self.settings_open = true;
                        }
                    });
                });
            });

        egui::SidePanel::left("activity-bar")
            .exact_width(48.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(theme.ui.background.0)
                    .inner_margin(egui::Margin::symmetric(4.0, 6.0))
                    .stroke(egui::Stroke::new(
                        1.0,
                        theme.ui.muted.0.gamma_multiply(0.22),
                    )),
            )
            .show(context, |ui| {
                let explorer = activity_button(ui, ActivityIcon::Explorer, self.files_open, &theme)
                    .on_hover_text(if self.files_open {
                        "Hide Explorer"
                    } else {
                        "Show Explorer"
                    });
                if explorer.clicked() {
                    self.files_open = !self.files_open;
                }
                if activity_button(ui, ActivityIcon::Preview, self.preview_open, &theme)
                    .on_hover_text("Toggle mathematical preview")
                    .clicked()
                {
                    self.preview_open = !self.preview_open;
                }
                if activity_button(ui, ActivityIcon::Problems, self.diagnostics_open, &theme)
                    .on_hover_text("Toggle diagnostics")
                    .clicked()
                {
                    self.diagnostics_open = !self.diagnostics_open;
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if activity_button(ui, ActivityIcon::Settings, self.settings_open, &theme)
                        .on_hover_text("Settings")
                        .clicked()
                    {
                        self.settings_open = true;
                    }
                });
            });

        if self.files_open {
            egui::SidePanel::left("files")
                .default_width(240.0)
                .width_range(180.0..=480.0)
                .resizable(true)
                .frame(
                    egui::Frame::none()
                        .fill(theme.ui.panel.0)
                        .inner_margin(egui::Margin::symmetric(8.0, 7.0))
                        .stroke(egui::Stroke::new(
                            1.0,
                            theme.ui.muted.0.gamma_multiply(0.20),
                        )),
                )
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("EXPLORER").strong().size(11.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button("Close")
                                .on_hover_text("Hide Explorer")
                                .clicked()
                            {
                                self.files_open = false;
                            }
                            if ui
                                .small_button("Refresh")
                                .on_hover_text("Refresh files")
                                .clicked()
                            {
                                self.files = collect_tree(&self.project_root);
                            }
                        });
                    });
                    let root_name = self
                        .project_root
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("workspace")
                        .to_uppercase();
                    ui.add_space(5.0);
                    ui.label(RichText::new(root_name).strong().size(11.0));
                    ui.separator();
                    let mut selected = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let mut hidden_below = None;
                        for entry in self.files.clone() {
                            if let Some(depth) = hidden_below {
                                if entry.depth > depth {
                                    continue;
                                }
                                hidden_below = None;
                            }
                            let collapsed =
                                entry.is_dir && self.collapsed_dirs.contains(&entry.path);
                            ui.horizontal(|ui| {
                                ui.add_space(entry.depth as f32 * 12.0);
                                let name = entry
                                    .path
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("?");
                                if entry.is_dir {
                                    let disclosure = tree_disclosure(ui, collapsed, &theme);
                                    paint_tree_icon(ui, true, &theme);
                                    let name_response = ui.add(
                                        egui::Label::new(
                                            RichText::new(name).strong().color(theme.ui.text.0),
                                        )
                                        .sense(egui::Sense::click()),
                                    );
                                    if disclosure.clicked() || name_response.clicked() {
                                        if collapsed {
                                            self.collapsed_dirs.remove(&entry.path);
                                        } else {
                                            self.collapsed_dirs.insert(entry.path.clone());
                                        }
                                    }
                                } else if ui
                                    .horizontal(|ui| {
                                        ui.add_space(14.0);
                                        paint_tree_icon(ui, false, &theme);
                                        ui.selectable_label(
                                            self.current_file.as_ref() == Some(&entry.path),
                                            name,
                                        )
                                    })
                                    .inner
                                    .clicked()
                                {
                                    selected = Some(entry.path.clone());
                                }
                            });
                            if collapsed {
                                hidden_below = Some(entry.depth);
                            }
                        }
                    });
                    if let Some(path) = selected {
                        self.open(path);
                    }
                });
        }

        egui::TopBottomPanel::bottom("status-bar")
            .exact_height(22.0)
            .frame(
                egui::Frame::none()
                    .fill(theme.ui.accent.0)
                    .inner_margin(egui::Margin::symmetric(8.0, 2.0)),
            )
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    let busy = self.run_result.is_some()
                        || self.analysis_result.is_some()
                        || self.analysis_due.is_some();
                    let status_mark = if busy { "WORKING" } else { "READY" };
                    let status_color = theme.contrast_on_accent();
                    ui.label(
                        RichText::new(status_mark)
                            .strong()
                            .size(11.0)
                            .color(status_color),
                    );
                    ui.label(RichText::new(&self.status).size(11.0).color(status_color));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let workspace_status = match self.workspace_tab {
                            WorkspaceTab::Document => format!(
                                "SCAS   UTF-8   Ln {}   Problems {}",
                                self.current_line + 1,
                                self.diagnostics.len()
                            ),
                            WorkspaceTab::Repl => {
                                format!("SCAS   REPL   Entries {}", self.repl_entries.len())
                            }
                            WorkspaceTab::Graphing => "SCAS   GRAPHING   Reserved".to_owned(),
                        };
                        ui.label(
                            RichText::new(workspace_status)
                                .size(11.0)
                                .color(status_color),
                        );
                    });
                });
            });

        if self.workspace_tab == WorkspaceTab::Document {
            egui::TopBottomPanel::bottom("output")
                .default_height(170.0)
                .height_range(80.0..=420.0)
                .resizable(true)
                .frame(
                    egui::Frame::none()
                        .fill(theme.ui.editor.0)
                        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                        .stroke(egui::Stroke::new(
                            1.0,
                            theme.ui.muted.0.gamma_multiply(0.22),
                        )),
                )
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(
                                !self.diagnostics_open,
                                RichText::new("OUTPUT").strong().size(11.0),
                            )
                            .clicked()
                        {
                            self.diagnostics_open = false;
                        }
                        if ui
                            .selectable_label(
                                self.diagnostics_open,
                                RichText::new(format!("PROBLEMS {}", self.diagnostics.len()))
                                    .strong()
                                    .size(11.0),
                            )
                            .clicked()
                        {
                            self.diagnostics_open = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !self.diagnostics_open && ui.small_button("Copy").clicked() {
                                ui.output_mut(|clipboard| {
                                    clipboard.copied_text.clone_from(&self.output)
                                });
                                self.set_status("Output copied");
                            }
                        });
                    });
                    ui.separator();
                    if self.diagnostics_open {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if self.diagnostics.is_empty() {
                                ui.label(
                                    RichText::new(if self.analysis_due.is_some() {
                                        "Checking the document after typing pauses…"
                                    } else {
                                        "No problems detected in this document."
                                    })
                                    .color(theme.ui.muted.0),
                                );
                            } else {
                                for diagnostic in &self.diagnostics {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new("ERROR")
                                                .strong()
                                                .size(10.5)
                                                .color(theme.syntax.operator.0),
                                        );
                                        ui.monospace(format!(
                                            "{}:{}  {}",
                                            diagnostic.line, diagnostic.column, diagnostic.message
                                        ));
                                    });
                                }
                            }
                        });
                    } else {
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(&self.output).monospace())
                                        .selectable(true)
                                        .extend(),
                                );
                            });
                    }
                });
        }

        if self.workspace_tab == WorkspaceTab::Document && self.preview_open {
            let preview_width = (context.screen_rect().width() - 48.0) * 0.46;
            egui::SidePanel::right("latex-preview")
                .default_width(preview_width)
                .width_range(280.0..=900.0)
                .resizable(true)
                .frame(
                    egui::Frame::none()
                        .fill(theme.ui.panel.0)
                        .inner_margin(egui::Margin::symmetric(10.0, 7.0))
                        .stroke(egui::Stroke::new(
                            1.0,
                            theme.ui.muted.0.gamma_multiply(0.22),
                        )),
                )
                .show(context, |ui| {
                    let mut enable_system_preview = false;
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("DOCUMENT PREVIEW")
                                .strong()
                                .size(11.0)
                                .color(self.themes[self.selected_theme].ui.text.0),
                        );
                        let state = if self.analysis_due.is_some() || self.analysis_result.is_some()
                        {
                            "UPDATING"
                        } else {
                            "LIVE"
                        };
                        ui.label(
                            RichText::new(state)
                                .size(10.5)
                                .color(self.themes[self.selected_theme].ui.accent.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Copy TeX").clicked() {
                                ui.output_mut(|output| {
                                    output.copied_text.clone_from(&self.preview)
                                });
                            }
                            let response = ui
                                .toggle_value(&mut self.system_latex_preview, "Computer Modern")
                                .on_hover_text("Use cached, serialized system LaTeX rendering");
                            enable_system_preview = response.changed() && self.system_latex_preview;
                        });
                    });
                    if enable_system_preview {
                        if let Some(document) = &self.preview_document {
                            let body = document.body();
                            let color = self.themes[self.selected_theme].ui.text.0;
                            self.start_latex_render(body, [color.r(), color.g(), color.b()]);
                        }
                    }
                    if let Some(error) = &self.preview_error {
                        ui.colored_label(
                            self.themes[self.selected_theme].syntax.operator.0,
                            format!("Editing: {error} (showing last good preview)"),
                        );
                    }
                    ui.separator();
                    if self
                        .preview_document
                        .as_ref()
                        .map_or(true, |document| document.fragments.is_empty())
                    {
                        ui.label("Add a render line, for example:");
                        ui.monospace(r"\[ (x + 1) / (2x^2) \]");
                    } else {
                        let preview_width = ui.available_width();
                        let preview_scale = (preview_width / 210.0).clamp(1.7, 3.2);
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if let Some(document) = &self.preview_document {
                                    if self.system_latex_preview {
                                        if let Some(texture) = &self.latex_texture {
                                            let natural_size = texture.size_vec2();
                                            let page_width = (preview_width - 52.0).max(180.0);
                                            let image_scale =
                                                (page_width / natural_size.x.max(1.0)).min(4.5);
                                            let display_size = natural_size * image_scale;
                                            ui.with_layout(
                                                egui::Layout::top_down(egui::Align::Min),
                                                |ui| {
                                                    ui.add_space(10.0);
                                                    ui.label(
                                                        RichText::new("Rendered document")
                                                            .strong()
                                                            .size(15.0),
                                                    );
                                                    ui.label(
                                                        RichText::new(
                                                            "Live output from the current SCAS source",
                                                        )
                                                        .size(11.0)
                                                        .color(theme.ui.muted.0),
                                                    );
                                                    ui.add_space(8.0);
                                                    egui::Frame::none()
                                                        .fill(theme.ui.editor.0)
                                                        .inner_margin(egui::Margin::same(18.0))
                                                        .show(ui, |ui| {
                                                            ui.add(
                                                                egui::Image::from_texture(texture)
                                                                    .fit_to_exact_size(
                                                                        display_size,
                                                                    ),
                                                            );
                                                        });
                                                },
                                            );
                                        } else {
                                            math_preview::show_document(
                                                ui,
                                                document,
                                                self.themes[self.selected_theme].ui.text.0,
                                                self.themes[self.selected_theme].ui.muted.0,
                                                preview_scale,
                                            );
                                        }
                                    } else {
                                        math_preview::show_document(
                                            ui,
                                            document,
                                            self.themes[self.selected_theme].ui.text.0,
                                            self.themes[self.selected_theme].ui.muted.0,
                                            preview_scale,
                                        );
                                    }
                                    if self.latex_render.is_some() {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.small("Typesetting Computer Modern preview…");
                                        });
                                    }
                                    if let Some(error) = &self.latex_render_error {
                                        ui.colored_label(
                                            self.themes[self.selected_theme].syntax.operator.0,
                                            format!("{error}; showing structured fallback"),
                                        );
                                    }
                                    for diagnostic in &document.diagnostics {
                                        ui.colored_label(
                                            self.themes[self.selected_theme].syntax.operator.0,
                                            &diagnostic.message,
                                        );
                                    }
                                }
                                ui.add_space(12.0);
                                ui.collapsing("LaTeX source", |ui| {
                                    ui.add(
                                        TextEdit::multiline(&mut self.preview)
                                            .font(TextStyle::Monospace)
                                            .desired_width(f32::INFINITY)
                                            .interactive(false),
                                    );
                                });
                            });
                    }
                });
        }

        let editor_background = self.themes[self.selected_theme].ui.editor.0;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(editor_background)
                    .inner_margin(egui::Margin::same(0.0)),
            )
            .show(context, |ui| {
                let editor_id = egui::Id::new(("source-editor", self.buffer_generation));
                let mut chosen_snippet = None;
                let document_title = self.title();
                egui::Frame::none()
                    .fill(self.themes[self.selected_theme].ui.panel.0)
                    .inner_margin(egui::Margin::symmetric(10.0, 5.0))
                    .stroke(egui::Stroke::new(
                        1.0,
                        self.themes[self.selected_theme]
                            .ui
                            .muted
                            .0
                            .gamma_multiply(0.20),
                    ))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (tab, label) in [
                                (WorkspaceTab::Document, document_title.as_str()),
                                (WorkspaceTab::Repl, "REPL"),
                                (WorkspaceTab::Graphing, "Graphing"),
                            ] {
                                let response = ui.selectable_label(
                                    self.workspace_tab == tab,
                                    RichText::new(label).strong().size(12.0),
                                );
                                if self.workspace_tab == tab {
                                    ui.painter().line_segment(
                                        [response.rect.left_bottom(), response.rect.right_bottom()],
                                        egui::Stroke::new(2.0, theme.ui.accent.0),
                                    );
                                }
                                if response.clicked() {
                                    self.workspace_tab = tab;
                                    if tab == WorkspaceTab::Repl {
                                        context.memory_mut(|memory| {
                                            memory.request_focus(egui::Id::new("gui-repl-input"))
                                        });
                                    }
                                }
                            }
                            if self.workspace_tab == WorkspaceTab::Document && self.dirty {
                                ui.label(
                                    RichText::new("Modified").size(11.0).color(theme.ui.muted.0),
                                );
                            }
                            if self.workspace_tab == WorkspaceTab::Document {
                                ui.label(
                                    RichText::new("SCAS")
                                        .size(11.0)
                                        .color(self.themes[self.selected_theme].ui.muted.0),
                                );
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if self.workspace_tab == WorkspaceTab::Repl {
                                        if ui.small_button("Reset session").clicked() {
                                            self.reset_repl();
                                        }
                                        if ui.small_button("Clear history").clicked() {
                                            self.repl_entries.clear();
                                            self.set_status("REPL history cleared");
                                        }
                                        return;
                                    }
                                    if self.workspace_tab != WorkspaceTab::Document {
                                        return;
                                    }
                                    if ui
                                        .small_button("Close")
                                        .on_hover_text("Close editor")
                                        .clicked()
                                    {
                                        if self.dirty {
                                            self.set_status(
                                                "Save the modified document before closing",
                                            );
                                        } else {
                                            self.new_file();
                                        }
                                    }
                                    ui.menu_button("Snippets", |ui| {
                                        ui.set_min_width(250.0);
                                        for snippet in editor_assist::SNIPPETS {
                                            if ui
                                                .button(snippet.label)
                                                .on_hover_text(snippet.detail)
                                                .clicked()
                                            {
                                                chosen_snippet = Some(*snippet);
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                    ui.menu_button("Ω  Symbols", |ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            for (symbol, insertion, hint) in [
                                                ("π", "π", "\\pi"),
                                                ("φ", "φ", "\\phi"),
                                                ("θ", "θ", "\\theta"),
                                                ("λ", "λ", "\\lambda"),
                                                ("Σ", "Σ", "\\Sigma"),
                                                ("Γ", "Γ", "\\Gamma"),
                                                ("∫", "integral(", "Integral function"),
                                                ("∇", "gradient(", "Gradient function"),
                                            ] {
                                                if ui
                                                    .button(RichText::new(symbol).size(17.0))
                                                    .on_hover_text(hint)
                                                    .clicked()
                                                {
                                                    self.source.push_str(insertion);
                                                    self.dirty = true;
                                                    self.schedule_analysis(context);
                                                    ui.close_menu();
                                                }
                                            }
                                        });
                                    });
                                    let greek_label = if self.auto_greek {
                                        "Greek shortcuts: On"
                                    } else {
                                        "Greek shortcuts: Off"
                                    };
                                    ui.toggle_value(&mut self.auto_greek, greek_label)
                                        .on_hover_text(
                                            "Enabled by default: expand TeX-style Greek shortcuts",
                                        );
                                },
                            );
                        });
                    });
                match self.workspace_tab {
                    WorkspaceTab::Repl => {
                        self.show_repl(ui, context, &theme);
                        return;
                    }
                    WorkspaceTab::Graphing => {
                        self.show_graphing(ui, &theme);
                        return;
                    }
                    WorkspaceTab::Document => {}
                }
                if let Some(snippet) = chosen_snippet {
                    let insertion = snippet.insertion.text();
                    self.insert_editor_template(
                        context,
                        editor_id,
                        self.cursor_char_index,
                        self.cursor_char_index,
                        &insertion,
                    );
                }
                ui.separator();
                let matches = editor_assist::completions(&self.source, self.cursor_char_index);
                let editor_focused = context.memory(|memory| memory.has_focus(editor_id));
                let tab_accept = editor_focused
                    && matches.len > 0
                    && context.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                    });
                if let Some(completion) = tab_accept.then(|| matches.first()).flatten() {
                    let insertion = completion.insertion.text();
                    self.insert_editor_template(
                        context,
                        editor_id,
                        matches.start_char,
                        self.cursor_char_index,
                        &insertion,
                    );
                }
                let font_id = FontId::new(
                    self.editor_font_size,
                    if self.editor_monospace {
                        FontFamily::Monospace
                    } else {
                        FontFamily::Proportional
                    },
                );
                let cache_is_current = self.highlight_cache.as_ref().is_some_and(|cache| {
                    cache.source == self.source
                        && cache.theme == self.selected_theme
                        && cache.font_size_bits == self.editor_font_size.to_bits()
                        && cache.monospace == self.editor_monospace
                        && cache.diagnostics_revision == self.diagnostics_revision
                });
                if !cache_is_current {
                    self.highlight_cache = Some(HighlightCache {
                        source: self.source.clone(),
                        theme: self.selected_theme,
                        font_size_bits: self.editor_font_size.to_bits(),
                        monospace: self.editor_monospace,
                        diagnostics_revision: self.diagnostics_revision,
                        job: highlight(
                            &self.source,
                            &self.themes[self.selected_theme],
                            font_id.clone(),
                            &self.diagnostics,
                        ),
                    });
                }
                let cache = self
                    .highlight_cache
                    .as_ref()
                    .expect("highlight cache was populated");
                let cached_source = cache.source.as_str();
                let cached_job = &cache.job;
                let fallback_theme = &self.themes[self.selected_theme];
                let fallback_font = &font_id;
                let fallback_diagnostics = &self.diagnostics;
                let mut layouter = |ui: &egui::Ui, source: &str, wrap_width: f32| {
                    let mut job = if source == cached_source {
                        cached_job.clone()
                    } else {
                        highlight(
                            source,
                            fallback_theme,
                            fallback_font.clone(),
                            fallback_diagnostics,
                        )
                    };
                    job.wrap.max_width = wrap_width;
                    ui.fonts(|fonts| fonts.layout_job(job))
                };
                let total_lines = self.source.split('\n').count().max(1);
                let digits = total_lines.to_string().len() as f32;
                let gutter_width = if self.line_numbers == LineNumberMode::Off {
                    4.0
                } else {
                    20.0 + digits * self.editor_font_size * 0.62
                };
                if TextEdit::load_state(context, editor_id).is_none() {
                    let mut state = egui::text_edit::TextEditState::default();
                    state.set_undoer(editor_undoer(&self.source));
                    TextEdit::store_state(context, editor_id, state);
                }
                let output = egui::ScrollArea::vertical()
                    .id_source("source-editor-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        TextEdit::multiline(&mut self.source)
                            .id(editor_id)
                            .font(font_id.clone())
                            .code_editor()
                            .frame(false)
                            .desired_width(ui.available_width())
                            .lock_focus(true)
                            .margin(egui::Margin {
                                left: gutter_width,
                                right: 4.0,
                                top: 4.0,
                                bottom: 4.0,
                            })
                            .layouter(&mut layouter)
                            .desired_rows(total_lines.max(1))
                            .show(ui)
                    })
                    .inner;
                if let Some(cursor) = output.cursor_range {
                    self.cursor_char_index = cursor.primary.ccursor.index;
                    self.current_line = self
                        .source
                        .chars()
                        .take(self.cursor_char_index)
                        .filter(|character| *character == '\n')
                        .count()
                        .min(total_lines.saturating_sub(1));
                }
                let popup_matches =
                    editor_assist::completions(&self.source, self.cursor_char_index);
                let mut popup_completion = None;
                if output.response.has_focus() && popup_matches.len > 0 {
                    if let Some(cursor) = output.cursor_range {
                        let caret = output.galley.pos_from_cursor(&cursor.primary);
                        let anchor =
                            output.galley_pos + egui::vec2(caret.left(), caret.bottom() + 4.0);
                        egui::Area::new(egui::Id::new("editor-completion-popup"))
                            .order(egui::Order::Foreground)
                            .fixed_pos(anchor)
                            .movable(false)
                            .constrain(true)
                            .show(context, |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    ui.set_min_width(290.0);
                                    for (index, assist) in popup_matches.iter().enumerate() {
                                        ui.horizontal(|ui| {
                                            let label = if index == 0 {
                                                format!("{}   Tab", assist.label)
                                            } else {
                                                assist.label.to_owned()
                                            };
                                            if ui.selectable_label(false, label).clicked() {
                                                popup_completion = Some(assist);
                                            }
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(assist.detail)
                                                            .size(11.0)
                                                            .color(
                                                                self.themes[self.selected_theme]
                                                                    .ui
                                                                    .muted
                                                                    .0,
                                                            ),
                                                    );
                                                },
                                            );
                                        });
                                    }
                                });
                            });
                    }
                }
                if self.line_numbers != LineNumberMode::Off {
                    paint_line_numbers(
                        ui,
                        &output,
                        self.current_line,
                        self.line_numbers,
                        &self.themes[self.selected_theme],
                        self.editor_font_size,
                    );
                }
                if let Some(pointer) = output.response.hover_pos() {
                    let cursor = output.galley.cursor_from_pos(pointer - output.galley_pos);
                    let byte = char_to_byte(&self.source, cursor.ccursor.index);
                    if let Some(diagnostic) = self
                        .diagnostics
                        .iter()
                        .find(|item| item.byte_start <= byte && byte < item.byte_end)
                    {
                        output.response.clone().on_hover_text(&diagnostic.message);
                    }
                }
                if output.response.changed() {
                    self.highlight_cache = None;
                    if self.auto_greek {
                        let before_expansion = self.source.clone();
                        expand_greek_shortcuts(&mut self.source);
                        if self.source != before_expansion {
                            if let Some(cursor) = output.cursor_range {
                                let mut state = output.state.clone();
                                let mut undoer = state.undoer();
                                undoer.add_undo(&(cursor.as_ccursor_range(), before_expansion));
                                state.set_undoer(undoer);
                                TextEdit::store_state(context, editor_id, state);
                            }
                        }
                    }
                    self.dirty = true;
                    self.schedule_analysis(context);
                }
                if let Some(completion) = popup_completion {
                    let insertion = completion.insertion.text();
                    self.insert_editor_template(
                        context,
                        editor_id,
                        popup_matches.start_char,
                        self.cursor_char_index,
                        &insertion,
                    );
                }
            });
    }
}

#[allow(dead_code)]
fn readable_latex(latex: &str) -> String {
    fn group(text: &str, start: usize) -> Option<(&str, usize)> {
        let mut depth = 0usize;
        for (offset, character) in text[start..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some((&text[start + 1..start + offset], start + offset + 1));
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn superscript(text: &str) -> String {
        text.chars()
            .map(|character| match character {
                '0' => '⁰',
                '1' => '¹',
                '2' => '²',
                '3' => '³',
                '4' => '⁴',
                '5' => '⁵',
                '6' => '⁶',
                '7' => '⁷',
                '8' => '⁸',
                '9' => '⁹',
                '+' => '⁺',
                '-' => '⁻',
                'n' => 'ⁿ',
                _ => character,
            })
            .collect()
    }

    let commands = [
        (r"\upalpha", "α"),
        (r"\upbeta", "β"),
        (r"\upgamma", "γ"),
        (r"\updelta", "δ"),
        (r"\upepsilon", "ε"),
        (r"\uptheta", "θ"),
        (r"\uplambda", "λ"),
        (r"\upmu", "μ"),
        (r"\uppi", "π"),
        (r"\uprho", "ρ"),
        (r"\upsigma", "σ"),
        (r"\upphi", "φ"),
        (r"\upomega", "ω"),
        (r"\alpha", "α"),
        (r"\beta", "β"),
        (r"\gamma", "γ"),
        (r"\delta", "δ"),
        (r"\theta", "θ"),
        (r"\lambda", "λ"),
        (r"\mu", "μ"),
        (r"\pi", "π"),
        (r"\rho", "ρ"),
        (r"\sigma", "σ"),
        (r"\phi", "φ"),
        (r"\omega", "ω"),
        (r"\Gamma", "Γ"),
        (r"\Delta", "Δ"),
        (r"\Theta", "Θ"),
        (r"\Lambda", "Λ"),
        (r"\Sigma", "Σ"),
        (r"\Phi", "Φ"),
        (r"\Omega", "Ω"),
        (r"\cdot", "·"),
        (r"\times", "×"),
        (r"\le", "≤"),
        (r"\ge", "≥"),
        (r"\ne", "≠"),
        (r"\land", "∧"),
        (r"\lor", "∨"),
        (r"\infty", "∞"),
        (r"\sin", "sin"),
        (r"\cos", "cos"),
        (r"\tan", "tan"),
        (r"\ln", "ln"),
        (r"\log", "log"),
        (r"\exp", "exp"),
    ];
    let mut output = String::new();
    let mut cursor = 0usize;
    while cursor < latex.len() {
        let rest = &latex[cursor..];
        if rest.starts_with(r"\mathrm{") {
            let start = cursor + r"\mathrm".len();
            if let Some((identifier, after)) = group(latex, start) {
                output.push_str(&readable_latex(identifier));
                cursor = after;
                continue;
            }
        }
        if rest.starts_with(r"\frac{") {
            let numerator_start = cursor + r"\frac".len();
            if let Some((numerator, after_numerator)) = group(latex, numerator_start) {
                if latex[after_numerator..].starts_with('{') {
                    if let Some((denominator, after_denominator)) = group(latex, after_numerator) {
                        output.push('(');
                        output.push_str(&readable_latex(numerator));
                        output.push_str(")⁄(");
                        output.push_str(&readable_latex(denominator));
                        output.push(')');
                        cursor = after_denominator;
                        continue;
                    }
                }
            }
        }
        if rest.starts_with(r"\sqrt{") {
            let start = cursor + r"\sqrt".len();
            if let Some((radicand, after)) = group(latex, start) {
                output.push_str("√(");
                output.push_str(&readable_latex(radicand));
                output.push(')');
                cursor = after;
                continue;
            }
        }
        if rest.starts_with("^{") {
            if let Some((exponent, after)) = group(latex, cursor + 1) {
                output.push_str(&superscript(&readable_latex(exponent)));
                cursor = after;
                continue;
            }
        }
        if let Some((command, replacement)) = commands.iter().find(|(command, _)| {
            rest.starts_with(command)
                && !rest[command.len()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphabetic)
        }) {
            output.push_str(replacement);
            cursor += command.len();
            continue;
        }
        if rest.starts_with(r"\left") || rest.starts_with(r"\right") {
            cursor += if rest.starts_with(r"\left") { 5 } else { 6 };
            continue;
        }
        if rest.starts_with(r"\[") || rest.starts_with(r"\]") {
            cursor += 2;
            continue;
        }
        if rest.starts_with(r"\,") {
            output.push(' ');
            cursor += 2;
            continue;
        }
        let character = rest.chars().next().expect("cursor is inside text");
        if character != '{' && character != '}' {
            output.push(character);
        }
        cursor += character.len_utf8();
    }
    output.trim().to_owned()
}

fn activity_button(
    ui: &mut egui::Ui,
    icon: ActivityIcon,
    selected: bool,
    theme: &Theme,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(40.0, 38.0), egui::Sense::click());
    let fill = if selected {
        theme.ui.selection.0
    } else if response.hovered() {
        theme.ui.panel.0
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 2.0, fill);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            2.0,
            egui::Stroke::new(1.0, theme.ui.accent.0),
        );
    }
    let color = if selected || response.hovered() {
        theme.ui.text.0
    } else {
        theme.ui.muted.0
    };
    let stroke = egui::Stroke::new(1.6, color);
    let center = rect.center();
    let icon_rect = egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0));
    match icon {
        ActivityIcon::Explorer => {
            let back = icon_rect.translate(egui::vec2(-2.5, -2.5)).shrink(2.0);
            let front = icon_rect.translate(egui::vec2(2.5, 2.5)).shrink(2.0);
            ui.painter().rect_stroke(back, 1.0, stroke);
            ui.painter().rect_stroke(front, 1.0, stroke);
        }
        ActivityIcon::Preview => {
            let page = icon_rect.shrink(2.5);
            ui.painter().rect_stroke(page, 1.0, stroke);
            for y in [page.top() + 5.0, page.top() + 9.0, page.top() + 13.0] {
                ui.painter().line_segment(
                    [
                        egui::pos2(page.left() + 3.0, y),
                        egui::pos2(page.right() - 3.0, y),
                    ],
                    egui::Stroke::new(1.1, color),
                );
            }
        }
        ActivityIcon::Problems => {
            let top = egui::pos2(center.x, icon_rect.top() + 1.5);
            let left = egui::pos2(icon_rect.left() + 1.0, icon_rect.bottom() - 2.0);
            let right = egui::pos2(icon_rect.right() - 1.0, icon_rect.bottom() - 2.0);
            ui.painter().line_segment([top, left], stroke);
            ui.painter().line_segment([left, right], stroke);
            ui.painter().line_segment([right, top], stroke);
            ui.painter().line_segment(
                [center - egui::vec2(0.0, 3.5), center + egui::vec2(0.0, 2.0)],
                stroke,
            );
            ui.painter()
                .circle_filled(center + egui::vec2(0.0, 6.0), 1.1, color);
        }
        ActivityIcon::Settings => {
            ui.painter().circle_stroke(center, 6.0, stroke);
            ui.painter().circle_stroke(center, 2.2, stroke);
            for step in 0..8 {
                let angle = step as f32 * std::f32::consts::TAU / 8.0;
                let direction = egui::vec2(angle.cos(), angle.sin());
                ui.painter()
                    .line_segment([center + direction * 7.0, center + direction * 9.5], stroke);
            }
        }
    }
    response
}

fn tree_disclosure(ui: &mut egui::Ui, collapsed: bool, theme: &Theme) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::click());
    let center = rect.center();
    let points = if collapsed {
        vec![
            center + egui::vec2(-2.0, -4.0),
            center + egui::vec2(3.0, 0.0),
            center + egui::vec2(-2.0, 4.0),
        ]
    } else {
        vec![
            center + egui::vec2(-4.0, -2.0),
            center + egui::vec2(4.0, -2.0),
            center + egui::vec2(0.0, 3.0),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        if response.hovered() {
            theme.ui.text.0
        } else {
            theme.ui.muted.0
        },
        egui::Stroke::NONE,
    ));
    response
}

fn paint_tree_icon(ui: &mut egui::Ui, directory: bool, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::hover());
    let color = theme.ui.muted.0;
    if directory {
        let body = egui::Rect::from_min_max(
            rect.left_top() + egui::vec2(1.0, 6.0),
            rect.right_bottom() - egui::vec2(1.0, 2.0),
        );
        ui.painter()
            .rect_stroke(body, 1.0, egui::Stroke::new(1.2, color));
        ui.painter().line_segment(
            [
                egui::pos2(body.left() + 1.0, body.top()),
                egui::pos2(body.left() + 6.0, body.top() - 3.0),
            ],
            egui::Stroke::new(1.2, color),
        );
    } else {
        let page = rect.shrink2(egui::vec2(3.0, 2.5));
        ui.painter()
            .rect_stroke(page, 1.0, egui::Stroke::new(1.1, color));
        ui.painter().line_segment(
            [
                egui::pos2(page.left() + 2.0, page.top() + 5.0),
                egui::pos2(page.right() - 2.0, page.top() + 5.0),
            ],
            egui::Stroke::new(1.0, color),
        );
    }
}

fn paint_line_numbers(
    ui: &egui::Ui,
    output: &egui::text_edit::TextEditOutput,
    current_line: usize,
    mode: LineNumberMode,
    theme: &Theme,
    font_size: f32,
) {
    // `text_clip_rect` excludes TextEdit's margin, which is exactly where the
    // gutter lives. Clip to the whole widget so the numbers are not discarded.
    let painter = ui
        .painter()
        .with_clip_rect(output.response.rect.intersect(ui.clip_rect()));
    let separator_x = output.galley_pos.x - 10.0;
    let gutter_rect = egui::Rect::from_min_max(
        output.response.rect.left_top(),
        egui::pos2(separator_x, output.response.rect.bottom()),
    );
    painter.rect_filled(gutter_rect, 0.0, theme.ui.editor.0);
    painter.line_segment(
        [
            egui::pos2(separator_x, output.text_clip_rect.top()),
            egui::pos2(separator_x, output.text_clip_rect.bottom()),
        ],
        egui::Stroke::new(1.0, theme.ui.muted.0.gamma_multiply(0.35)),
    );
    let mut source_line = 0usize;
    let mut first_visual_row = true;
    for galley_row in &output.galley.rows {
        let active = source_line == current_line;
        if active {
            let row_rect = egui::Rect::from_min_max(
                egui::pos2(
                    separator_x + 1.0,
                    output.galley_pos.y + galley_row.rect.top(),
                ),
                egui::pos2(
                    output.text_clip_rect.right(),
                    output.galley_pos.y + galley_row.rect.bottom(),
                ),
            );
            painter.rect_filled(row_rect, 0.0, theme.ui.accent.0.gamma_multiply(0.075));
        }
        let value = first_visual_row
            .then(|| displayed_line_number(mode, source_line, current_line))
            .flatten();
        if let Some(value) = value {
            let position = egui::pos2(
                separator_x - 8.0,
                output.galley_pos.y + galley_row.rect.center().y,
            );
            painter.text(
                position,
                egui::Align2::RIGHT_CENTER,
                value.to_string(),
                FontId::monospace(font_size * 0.82),
                if active {
                    theme.ui.accent.0
                } else {
                    theme.ui.text.0.gamma_multiply(0.72)
                },
            );
        }
        first_visual_row = galley_row.ends_with_newline;
        if galley_row.ends_with_newline {
            source_line += 1;
        }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Highlight {
    Plain,
    Keyword,
    Number,
    String,
    Comment,
    DocComment,
    Identifier,
    Operator,
    Delimiter,
    RenderMarker,
}

#[derive(Clone, Copy, Debug)]
struct HighlightSpan {
    start: usize,
    end: usize,
    kind: Highlight,
}

fn highlight(
    source: &str,
    theme: &Theme,
    font_id: FontId,
    diagnostics: &[EditorDiagnostic],
) -> LayoutJob {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    let mut spans = tokens
        .iter()
        .filter_map(|token| {
            token_highlight(token.token_type).map(|kind| HighlightSpan {
                start: token.byte_start,
                end: token.byte_end,
                kind,
            })
        })
        .collect::<Vec<_>>();

    // Ordinary comments are trivia by design, so the lexer does not emit them.
    // Recover only those gaps here; token spans prevent `//` inside strings from
    // being mistaken for a comment.
    for (line_start, line) in source.split_inclusive('\n').scan(0, |offset, line| {
        let start = *offset;
        *offset += line.len();
        Some((start, line))
    }) {
        let bytes = line.as_bytes();
        let mut index = 0;
        while index + 1 < bytes.len() {
            if bytes[index] == b'/' && bytes[index + 1] == b'/' {
                let candidate = line_start + index;
                let covered = spans
                    .iter()
                    .any(|span| span.start <= candidate && candidate < span.end);
                if !covered {
                    let end = line_start + line.trim_end_matches('\n').len();
                    spans.push(HighlightSpan {
                        start: candidate,
                        end,
                        kind: Highlight::Comment,
                    });
                    break;
                }
            }
            index += 1;
        }
    }
    spans.sort_by_key(|span| (span.start, span.end));

    let mut job = LayoutJob::default();
    let mut cursor = 0;
    for span in spans {
        if span.start < cursor || span.end > source.len() || span.start >= span.end {
            continue;
        }
        append_highlight(
            &mut job,
            &source[cursor..span.start],
            Highlight::Plain,
            theme,
            &font_id,
        );
        append_highlight(
            &mut job,
            &source[span.start..span.end],
            span.kind,
            theme,
            &font_id,
        );
        cursor = span.end;
    }
    append_highlight(
        &mut job,
        &source[cursor..],
        Highlight::Plain,
        theme,
        &font_id,
    );
    let diagnostic_color = theme.syntax.operator.0;
    let original_sections = std::mem::take(&mut job.sections);
    for section in original_sections {
        let mut boundaries = vec![section.byte_range.start, section.byte_range.end];
        for diagnostic in diagnostics {
            if section.byte_range.start < diagnostic.byte_start
                && diagnostic.byte_start < section.byte_range.end
            {
                boundaries.push(diagnostic.byte_start);
            }
            if section.byte_range.start < diagnostic.byte_end
                && diagnostic.byte_end < section.byte_range.end
            {
                boundaries.push(diagnostic.byte_end);
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        for range in boundaries.windows(2) {
            let mut part = section.clone();
            part.byte_range = range[0]..range[1];
            if diagnostics.iter().any(|diagnostic| {
                part.byte_range.start < diagnostic.byte_end
                    && diagnostic.byte_start < part.byte_range.end
            }) {
                part.format.underline = egui::Stroke::new(1.5, diagnostic_color);
            }
            job.sections.push(part);
        }
    }
    job
}

fn source_diagnostics(source: &str) -> Vec<EditorDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    for error in scanner.errors() {
        let start = byte_offset(source, error.line, error.column);
        let end = byte_end_for_chars(source, start, error.length.max(1));
        diagnostics.push(EditorDiagnostic {
            byte_start: start,
            byte_end: end,
            line: error.line,
            column: error.column,
            message: error.message.clone(),
        });
    }
    if diagnostics.is_empty() {
        for error in Parser::new(tokens).parse_recovering().errors {
            let start = byte_offset(source, error.line, error.column);
            let end = byte_end_for_chars(source, start, error.length.max(1));
            diagnostics.push(EditorDiagnostic {
                byte_start: start,
                byte_end: end,
                line: error.line,
                column: error.column,
                message: error.message,
            });
        }
    }
    diagnostics
}

fn byte_offset(source: &str, line: usize, column: usize) -> usize {
    let line_start = source
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>()
        .min(source.len());
    line_start
        + source[line_start..]
            .chars()
            .take(column.saturating_sub(1))
            .map(char::len_utf8)
            .sum::<usize>()
}

fn byte_end_for_chars(source: &str, start: usize, count: usize) -> usize {
    let width = source[start.min(source.len())..]
        .chars()
        .take(count)
        .map(char::len_utf8)
        .sum::<usize>();
    (start + width.max(1)).min(source.len())
}

fn char_to_byte(source: &str, char_index: usize) -> usize {
    source
        .char_indices()
        .nth(char_index)
        .map_or(source.len(), |(index, _)| index)
}

fn append_highlight(
    job: &mut LayoutJob,
    text: &str,
    kind: Highlight,
    theme: &Theme,
    font_id: &FontId,
) {
    if text.is_empty() {
        return;
    }
    let syntax = &theme.syntax;
    let color = match kind {
        Highlight::Plain => syntax.plain.0,
        Highlight::Keyword => syntax.keyword.0,
        Highlight::Number => syntax.number.0,
        Highlight::String => syntax.string.0,
        Highlight::Comment => syntax.comment.0,
        Highlight::DocComment => syntax.doc_comment.0,
        Highlight::Identifier => syntax.identifier.0,
        Highlight::Operator => syntax.operator.0,
        Highlight::Delimiter => syntax.delimiter.0,
        Highlight::RenderMarker => syntax.render_marker.0,
    };
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font_id.clone(),
            color,
            ..Default::default()
        },
    );
}

fn token_highlight(token: TokenType) -> Option<Highlight> {
    use TokenType::*;
    Some(match token {
        Eof => return None,
        Let | Fn | If | Else | For | While | Break | Continue | Return | Print | And | Or | Xor
        | True | False | Nil | Exit | Help | Clear => Highlight::Keyword,
        Number => Highlight::Number,
        Str => Highlight::String,
        DocComment => Highlight::DocComment,
        Identifier => Highlight::Identifier,
        RenderLeftBrace | RenderRightBrace | RenderLeftBracket | RenderRightBracket => {
            Highlight::RenderMarker
        }
        LeftParen | RightParen | LeftBracket | RightBracket | LeftBrace | RightBrace | Comma
        | Dot | Colon | Semicolon => Highlight::Delimiter,
        FlowOperator | FatArrow | Plus | Minus | Star | Slash | Mod | Pow | Ampersand | Pipe
        | Tilde | ShiftLeft | ShiftRight | ShiftLeftEqual | ShiftRightEqual | Equal | PlusEqual
        | MinusEqual | StarEqual | SlashEqual | ModEqual | PowEqual | AmpersandEqual
        | PipeEqual | TildeEqual | EqualEqual | BangEqual | Less | LessEqual | Greater
        | Apostrophe | GreaterEqual | Bang => Highlight::Operator,
    })
}

fn collect_tree(root: &Path) -> Vec<TreeEntry> {
    let mut entries = Vec::new();
    visit_directory(root, 0, &mut entries);
    entries
}

fn visit_directory(directory: &Path, depth: usize, output: &mut Vec<TreeEntry>) {
    if depth > 12 {
        return;
    }
    let Ok(read_dir) = fs::read_dir(directory) else {
        return;
    };
    let mut paths = read_dir
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.retain(|path| {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        !name.starts_with('.') && name != "target" && name != "fuzz"
    });
    paths.sort_by_key(|path| (!path.is_dir(), path.file_name().map(|name| name.to_owned())));
    for path in paths {
        if path.is_dir() {
            output.push(TreeEntry {
                path: path.clone(),
                depth,
                is_dir: true,
            });
            visit_directory(&path, depth + 1, output);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "scas")
        {
            output.push(TreeEntry {
                path,
                depth,
                is_dir: false,
            });
        }
    }
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

fn editor_undoer(source: &str) -> egui::util::undoer::Undoer<(egui::text::CCursorRange, String)> {
    let mut undoer = egui::util::undoer::Undoer::with_settings(egui::util::undoer::Settings {
        max_undos: 200,
        // Record each completed input frame. A long typing burst or paste
        // must never leave the empty buffer as the only undo destination.
        stable_time: 0.0,
        auto_save_interval: 2.0,
    });
    undoer.add_undo(&(egui::text::CCursorRange::default(), source.to_owned()));
    undoer
}

fn live_preview_limits() -> Limits {
    Limits {
        max_steps: 120_000,
        max_call_depth: 96,
        max_exponent: 10_000,
        max_shift: 100_000,
        max_decimal_precision: 512,
        max_transcendental_precision: 256,
        max_number_bits: 1_000_000,
        ..Limits::default()
    }
}

fn render_latex_png(body: &str, color: [u8; 3]) -> Result<LatexRender, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("scas-latex-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create LaTeX preview directory: {error}"))?;
    let source = format!(
        "\\documentclass[preview,border=10pt]{{standalone}}\n\
         \\usepackage{{amsmath}}\n\
         \\usepackage{{amssymb}}\n\
         \\usepackage{{upgreek}}\n\
         \\usepackage{{booktabs}}\n\
         \\begin{{document}}\n{body}\n\\end{{document}}\n"
    );
    let result = (|| {
        fs::write(directory.join("preview.tex"), source)
            .map_err(|error| format!("could not write LaTeX preview: {error}"))?;
        let latex = Command::new("latex")
            .args(["-interaction=nonstopmode", "-halt-on-error", "preview.tex"])
            .current_dir(&directory)
            .output()
            .map_err(|error| format!("could not start LaTeX: {error}"))?;
        if !latex.status.success() {
            return Err("LaTeX could not typeset the current document".to_owned());
        }
        let foreground = format!(
            "rgb {:.4} {:.4} {:.4}",
            f32::from(color[0]) / 255.0,
            f32::from(color[1]) / 255.0,
            f32::from(color[2]) / 255.0
        );
        let dvipng = Command::new("dvipng")
            .args([
                "-T",
                "tight",
                "-D",
                "180",
                "-bg",
                "Transparent",
                "-fg",
                &foreground,
                "-o",
                "preview.png",
                "preview.dvi",
            ])
            .current_dir(&directory)
            .output()
            .map_err(|error| format!("could not start dvipng: {error}"))?;
        if !dvipng.status.success() {
            return Err("dvipng could not rasterize the LaTeX preview".to_owned());
        }
        let bytes = fs::read(directory.join("preview.png"))
            .map_err(|error| format!("could not read LaTeX preview: {error}"))?;
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|error| format!("could not decode LaTeX preview: {error}"))?
            .to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        Ok(LatexRender {
            rgba: image.into_raw(),
            size,
        })
    })();
    let _ = fs::remove_dir_all(&directory);
    result
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "SCAS",
        options,
        Box::new(|context| Ok(Box::new(ScasApp::new(context)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color_at(job: &LayoutJob, needle: &str) -> egui::Color32 {
        let offset = job.text.find(needle).unwrap();
        job.sections
            .iter()
            .find(|section| section.byte_range.contains(&offset))
            .unwrap()
            .format
            .color
    }

    #[test]
    fn expands_supported_greek_shortcuts_without_touching_render_markers() {
        let mut source =
            "let angle = \\theta; print(\\pi); let variants = [\\varphi, \\Xi]; \\[ angle \\]"
                .to_owned();
        expand_greek_shortcuts(&mut source);
        assert_eq!(
            source,
            "let angle = θ; print(π); let variants = [ϕ, Ξ]; \\[ angle \\]"
        );
    }

    #[test]
    fn editor_undo_is_seeded_with_the_loaded_document() {
        let initial = "first line\nsecond line";
        let mut undoer = editor_undoer(initial);
        let edited = (egui::text::CCursorRange::default(), format!("{initial}!"));
        undoer.add_undo(&edited);
        let restored = undoer.undo(&edited).expect("one edit should be undoable");
        assert_eq!(restored.1, initial);
    }

    #[test]
    fn highlights_lexer_tokens_and_distinguishes_render_markers() {
        let theme = Theme::bundled_themes().remove(0);
        let source = "let x = 42; \\[ x =>; \\]";
        let job = highlight(source, &theme, FontId::monospace(15.0), &[]);
        assert_eq!(color_at(&job, "let"), theme.syntax.keyword.0);
        assert_eq!(color_at(&job, "42"), theme.syntax.number.0);
        assert_eq!(color_at(&job, "\\["), theme.syntax.render_marker.0);
        assert_ne!(color_at(&job, "\\["), theme.syntax.delimiter.0);
    }

    #[test]
    fn highlights_doc_and_ordinary_comments_without_coloring_slashes_in_strings() {
        let theme = Theme::bundled_themes().remove(0);
        let source = "/// Math prose\nlet url = \"https://example.test\"; // note\n";
        let job = highlight(source, &theme, FontId::monospace(15.0), &[]);
        assert_eq!(color_at(&job, "///"), theme.syntax.doc_comment.0);
        assert_eq!(color_at(&job, "// note"), theme.syntax.comment.0);
        assert_eq!(color_at(&job, "https"), theme.syntax.string.0);
    }

    #[test]
    fn live_preview_assembles_prose_and_rendered_expressions() {
        let preview =
            scas::document::build("/// Quadratic formula\n\\[ (x + 1) / (2x^2) \\]\nprint(x);\n")
                .unwrap()
                .body();
        assert_eq!(
            preview,
            "Quadratic formula\n\n\\[\\frac{\\left(x + 1\\right)}{\\left(2x^{2}\\right)}\\]"
        );
        assert_eq!(
            readable_latex(&preview),
            "Quadratic formula\n\n((x + 1))⁄((2x²))"
        );
    }

    #[test]
    fn repl_input_is_rendered_from_the_live_ast() {
        let symbols = SymbolTable::new();
        let power = repl_expression_latex("(x + 1)^2", &symbols).unwrap();
        assert!(power.contains(r"^{2}"));
        assert!(power.contains("x + 1"));

        let matrix = repl_expression_latex("let A = [1, 2; 3, 4]", &symbols).unwrap();
        assert!(matrix.contains('='));
        assert!(matrix.contains(r"\begin{pmatrix}"));

        assert!(repl_expression_latex("(x +", &symbols).is_none());
    }

    #[test]
    fn starter_opens_with_a_typeset_document() {
        let document = scas::document::build(STARTER).unwrap();
        assert!(!document.fragments.is_empty());
        assert!(document.body().contains(r"\sin"));
        assert!(document.body().contains(r"\frac{1}{2}"));
    }

    #[test]
    fn diagnostic_ranges_are_unicode_aware_and_underlined() {
        let source = "let π = ;";
        let start = byte_offset(source, 1, 9);
        assert_eq!(&source[start..], ";");
        let diagnostic = EditorDiagnostic {
            byte_start: start,
            byte_end: start + 1,
            line: 1,
            column: 9,
            message: "expected expression".to_owned(),
        };
        let job = highlight(
            source,
            &Theme::bundled_themes()[0],
            FontId::monospace(15.0),
            &[diagnostic],
        );
        let semicolon = job.text.find(';').unwrap();
        assert!(job.sections.iter().any(|section| {
            section.byte_range.contains(&semicolon) && section.format.underline.width > 0.0
        }));
    }

    #[test]
    fn line_number_modes_produce_editor_gutter_values() {
        assert_eq!(
            displayed_line_number(LineNumberMode::Absolute, 4, 2),
            Some(5)
        );
        assert_eq!(
            displayed_line_number(LineNumberMode::Relative, 4, 2),
            Some(2)
        );
        assert_eq!(displayed_line_number(LineNumberMode::Hybrid, 2, 2), Some(3));
        assert_eq!(displayed_line_number(LineNumberMode::Hybrid, 4, 2), Some(2));
        assert_eq!(displayed_line_number(LineNumberMode::Off, 4, 2), None);
    }

    #[test]
    fn system_latex_produces_a_real_computer_modern_preview_when_available() {
        if Command::new("latex").arg("--version").output().is_err()
            || Command::new("dvipng").arg("--version").output().is_err()
        {
            return;
        }
        let document = scas::document::build(
            r#"\{
rref([1, 2; 3, 4]) =>;
solve(z^2-z, z) =>;
\}"#,
        )
        .unwrap();
        let rendered = render_latex_png(&document.body(), [20, 20, 20]).unwrap();
        assert!(rendered.size[0] > 20);
        assert!(rendered.size[1] > 20);
        assert_eq!(rendered.rgba.len(), rendered.size[0] * rendered.size[1] * 4);
    }
}
