#[path = "scas_gui/math_preview.rs"]
mod math_preview;
#[path = "scas_gui/theme.rs"]
mod theme;

use eframe::egui::{
    self, text::LayoutJob, FontData, FontDefinitions, FontFamily, FontId, RichText, TextEdit,
    TextFormat, TextStyle,
};
use scas::document::Document;
use scas::execute;
use scas::lexer::{Scanner, TokenType};
use scas::parser::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use theme::Theme;

const STARTER: &str = "/// A live, typeset result\n\
let angle = π / 6;\n\
\\[ sin(angle) => \\]\n\
print(sin(angle));\n";

const GREEK: &[(&str, &str)] = &[
    ("\\Gamma", "Γ"),
    ("\\Delta", "Δ"),
    ("\\Theta", "Θ"),
    ("\\Lambda", "Λ"),
    ("\\Sigma", "Σ"),
    ("\\Phi", "Φ"),
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
    ("\\pi", "π"),
    ("\\rho", "ρ"),
    ("\\sigma", "σ"),
    ("\\tau", "τ"),
    ("\\phi", "φ"),
    ("\\chi", "χ"),
    ("\\psi", "ψ"),
    ("\\omega", "ω"),
];

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
    latex_render: Option<Receiver<Result<LatexRender, String>>>,
    latex_texture: Option<egui::TextureHandle>,
    latex_render_error: Option<String>,
    line_numbers: LineNumberMode,
    current_line: usize,
}

impl ScasApp {
    fn new(context: &eframe::CreationContext<'_>) -> Self {
        let themes = vec![Theme::gruber_darker(), Theme::paper_light()];
        themes[0].apply(&context.egui_ctx);
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let files = collect_tree(&project_root);
        let mut app = Self {
            source: STARTER.to_owned(),
            output: "Run the program to see its output.".to_owned(),
            current_file: None,
            project_root,
            files,
            dirty: false,
            auto_greek: true,
            run_result: None,
            status: "Ready".to_owned(),
            themes,
            selected_theme: 0,
            editor_font_size: 15.0,
            editor_monospace: true,
            settings_open: false,
            theme_path: "themes/gruber-darker.toml".to_owned(),
            font_path: String::new(),
            preview: String::new(),
            preview_document: None,
            preview_error: None,
            preview_open: true,
            files_open: false,
            diagnostics_open: true,
            diagnostics: Vec::new(),
            analysis_due: None,
            analysis_result: None,
            analysis_generation: 0,
            buffer_generation: 0,
            latex_render: None,
            latex_texture: None,
            latex_render_error: None,
            line_numbers: LineNumberMode::Absolute,
            current_line: 0,
        };
        app.start_analysis();
        app
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

    fn new_file(&mut self) {
        self.source.clear();
        self.current_file = None;
        self.dirty = false;
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

    fn start_analysis(&mut self) {
        let source = self.source.clone();
        let generation = self.analysis_generation;
        let (sender, receiver) = mpsc::channel();
        self.analysis_result = Some(receiver);
        std::thread::spawn(move || {
            let diagnostics = source_diagnostics(&source);
            let document = scas::document::build(&source).map_err(|error| error.to_string());
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
                self.start_latex_render(
                    document.body(),
                    [text_color.r(), text_color.g(), text_color.b()],
                );
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
                self.preview_document = Some(document);
                self.preview_error = None;
            }
            Err(error) => {
                self.diagnostics = diagnostics;
                // Keep the last successful document visible while the user is
                // in the middle of typing an incomplete expression.
                self.preview_error = Some(error.to_string());
            }
        }
    }

    fn schedule_analysis(&mut self, context: &egui::Context) {
        const DEBOUNCE: Duration = Duration::from_millis(650);
        self.analysis_generation = self.analysis_generation.wrapping_add(1);
        self.analysis_due = Some(Instant::now() + DEBOUNCE);
        // Do not show stale errors while the user is repairing the source.
        self.diagnostics.clear();
        self.preview_error = None;
        context.request_repaint_after(DEBOUNCE);
    }

    fn poll_analysis(&mut self, context: &egui::Context) {
        if let Some(due) = self.analysis_due {
            let now = Instant::now();
            if now >= due {
                self.analysis_due = None;
                self.start_analysis();
            } else {
                context.request_repaint_after(due.duration_since(now));
            }
        }

        if let Some(receiver) = &self.analysis_result {
            match receiver.try_recv() {
                Ok(result) => {
                    self.analysis_result = None;
                    self.apply_analysis(result);
                }
                Err(TryRecvError::Empty) => {
                    context.request_repaint_after(Duration::from_millis(50));
                }
                Err(TryRecvError::Disconnected) => {
                    self.analysis_result = None;
                    self.preview_error = Some("analysis worker stopped unexpectedly".to_owned());
                }
            }
        }
    }

    fn start_latex_render(&mut self, body: String, color: [u8; 3]) {
        let (sender, receiver) = mpsc::channel();
        self.latex_render = Some(receiver);
        self.latex_render_error = None;
        std::thread::spawn(move || {
            let result = render_latex_png(&body, color);
            let _ = sender.send(result);
        });
    }

    fn poll_latex_render(&mut self, context: &egui::Context) {
        let Some(receiver) = &self.latex_render else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(render)) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(render.size, &render.rgba);
                self.latex_texture = Some(context.load_texture(
                    "latex-preview",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
                self.latex_render = None;
            }
            Ok(Err(error)) => {
                self.latex_render_error = Some(error);
                self.latex_render = None;
            }
            Err(TryRecvError::Empty) => context.request_repaint_after(Duration::from_millis(50)),
            Err(TryRecvError::Disconnected) => {
                self.latex_render_error = Some("LaTeX preview worker stopped".to_owned());
                self.latex_render = None;
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

    fn settings(&mut self, context: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
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
            self.run();
        }

        egui::TopBottomPanel::top("toolbar").show(context, |ui| {
            ui.horizontal(|ui| {
                if ui.button("New").clicked() {
                    self.new_file();
                }
                if ui.button("Save  Ctrl+S").clicked() {
                    self.save();
                }
                let running = self.run_result.is_some();
                if ui
                    .add_enabled(!running, egui::Button::new("▶ Run  Ctrl+Enter"))
                    .clicked()
                {
                    self.run();
                }
                ui.separator();
                ui.checkbox(&mut self.auto_greek, "Expand \\name");
                ui.checkbox(&mut self.preview_open, "LaTeX preview");
                if ui.button("Editor settings").clicked() {
                    self.settings_open = true;
                }
                ui.separator();
                ui.label(&self.status);
            });
        });

        egui::SidePanel::left("activity-bar")
            .exact_width(44.0)
            .resizable(false)
            .show(context, |ui| {
                ui.add_space(6.0);
                let explorer = ui
                    .selectable_label(self.files_open, RichText::new("▱").size(25.0))
                    .on_hover_text(if self.files_open {
                        "Hide Explorer"
                    } else {
                        "Show Explorer"
                    });
                if explorer.clicked() {
                    self.files_open = !self.files_open;
                }
            });

        if self.files_open {
            egui::SidePanel::left("files")
                .default_width(230.0)
                .width_range(140.0..=480.0)
                .resizable(true)
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("FILES");
                        if ui.small_button("↻").on_hover_text("Refresh").clicked() {
                            self.files = collect_tree(&self.project_root);
                        }
                        if ui
                            .small_button("×")
                            .on_hover_text("Hide file tree")
                            .clicked()
                        {
                            self.files_open = false;
                        }
                    });
                    ui.small(self.project_root.display().to_string());
                    ui.separator();
                    let mut selected = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for entry in &self.files {
                            ui.horizontal(|ui| {
                                ui.add_space(entry.depth as f32 * 12.0);
                                let name = entry
                                    .path
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("?");
                                if entry.is_dir {
                                    ui.label(
                                        RichText::new(format!("▾ {name}"))
                                            .color(self.themes[self.selected_theme].ui.muted.0),
                                    );
                                } else if ui
                                    .selectable_label(
                                        self.current_file.as_ref() == Some(&entry.path),
                                        format!("◇ {name}"),
                                    )
                                    .clicked()
                                {
                                    selected = Some(entry.path.clone());
                                }
                            });
                        }
                    });
                    if let Some(path) = selected {
                        self.open(path);
                    }
                });
        }

        egui::TopBottomPanel::bottom("output")
            .default_height(190.0)
            .height_range(90.0..=420.0)
            .resizable(true)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("OUTPUT");
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|clipboard| clipboard.copied_text.clone_from(&self.output));
                        self.set_status("Output copied");
                    }
                });
                ui.separator();
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
            });

        if self.preview_open {
            let preview_width = (context.screen_rect().width() - 44.0) * 0.5;
            egui::SidePanel::right("latex-preview")
                .default_width(preview_width)
                .width_range(280.0..=900.0)
                .resizable(true)
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("LATEX PREVIEW");
                        if ui.small_button("Copy").clicked() {
                            ui.output_mut(|output| output.copied_text.clone_from(&self.preview));
                        }
                    });
                    ui.small("Live document output from /// prose and \\[ expression \\] lines");
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
                        let preview_scale = (preview_width / 340.0).clamp(0.55, 1.35);
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if let Some(document) = &self.preview_document {
                                    if let Some(texture) = &self.latex_texture {
                                        ui.add(
                                            egui::Image::from_texture(texture)
                                                .fit_to_original_size(0.8)
                                                .max_width(preview_width),
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
                                    if self.latex_render.is_some() {
                                        ui.small("Typesetting with LaTeX…");
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
                    .inner_margin(egui::Margin::same(8.0)),
            )
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(self.title());
                    ui.separator();
                    ui.label("Symbols:");
                    for (symbol, hint) in [
                        ("π", "\\pi"),
                        ("γ", "\\gamma"),
                        ("θ", "\\theta"),
                        ("λ", "\\lambda"),
                        ("Σ", "\\Sigma"),
                        ("Γ", "\\Gamma"),
                    ] {
                        if ui.small_button(symbol).on_hover_text(hint).clicked() {
                            self.source.push_str(symbol);
                            self.dirty = true;
                            self.schedule_analysis(context);
                        }
                    }
                });
                ui.separator();
                let theme = self.themes[self.selected_theme].clone();
                let font_id = FontId::new(
                    self.editor_font_size,
                    if self.editor_monospace {
                        FontFamily::Monospace
                    } else {
                        FontFamily::Proportional
                    },
                );
                let highlight_font = font_id.clone();
                let editor_diagnostics = self.diagnostics.clone();
                let mut layouter = move |ui: &egui::Ui, source: &str, _wrap_width: f32| {
                    let mut job =
                        highlight(source, &theme, highlight_font.clone(), &editor_diagnostics);
                    job.wrap.max_width = f32::INFINITY;
                    ui.fonts(|fonts| fonts.layout_job(job))
                };
                let total_lines = self.source.split('\n').count().max(1);
                let digits = total_lines.to_string().len() as f32;
                let gutter_width = if self.line_numbers == LineNumberMode::Off {
                    4.0
                } else {
                    20.0 + digits * self.editor_font_size * 0.62
                };
                if self.diagnostics_open {
                    egui::CollapsingHeader::new(format!(
                        "DIAGNOSTICS ({})",
                        self.diagnostics.len()
                    ))
                    .id_source("editor-diagnostics")
                    .default_open(false)
                    .show(ui, |ui| {
                        if self.diagnostics.is_empty() {
                            ui.weak(if self.analysis_due.is_some() {
                                "Checking after typing pauses…"
                            } else {
                                "No diagnostics"
                            });
                        } else {
                            for diagnostic in &self.diagnostics {
                                ui.colored_label(
                                    self.themes[self.selected_theme].syntax.operator.0,
                                    format!(
                                        "{}:{}  {}",
                                        diagnostic.line, diagnostic.column, diagnostic.message
                                    ),
                                );
                            }
                        }
                    });
                    ui.separator();
                }
                let editor_id = egui::Id::new(("source-editor", self.buffer_generation));
                if TextEdit::load_state(context, editor_id).is_none() {
                    let mut state = egui::text_edit::TextEditState::default();
                    state.set_undoer(egui::util::undoer::Undoer::with_settings(
                        egui::util::undoer::Settings {
                            max_undos: 200,
                            stable_time: 0.18,
                            auto_save_interval: 2.0,
                        },
                    ));
                    TextEdit::store_state(context, editor_id, state);
                }
                let output = egui::ScrollArea::both()
                    .id_source("source-editor-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        TextEdit::multiline(&mut self.source)
                            .id(editor_id)
                            .font(font_id)
                            .code_editor()
                            .frame(false)
                            .desired_width(ui.available_width().max(600.0))
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
                    self.current_line = cursor
                        .primary
                        .rcursor
                        .row
                        .min(total_lines.saturating_sub(1));
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
    if let Some(row) = output.galley.rows.get(current_line) {
        let row_rect = egui::Rect::from_min_max(
            egui::pos2(separator_x + 1.0, output.galley_pos.y + row.rect.top()),
            egui::pos2(
                output.text_clip_rect.right(),
                output.galley_pos.y + row.rect.bottom(),
            ),
        );
        painter.rect_filled(row_rect, 0.0, theme.ui.accent.0.gamma_multiply(0.075));
    }
    painter.line_segment(
        [
            egui::pos2(separator_x, output.text_clip_rect.top()),
            egui::pos2(separator_x, output.text_clip_rect.bottom()),
        ],
        egui::Stroke::new(1.0, theme.ui.muted.0.gamma_multiply(0.35)),
    );
    for (row, galley_row) in output.galley.rows.iter().enumerate() {
        let Some(value) = displayed_line_number(mode, row, current_line) else {
            continue;
        };
        let active = row == current_line;
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
        | GreaterEqual | Bang => Highlight::Operator,
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
    for (shortcut, symbol) in GREEK {
        *source = source.replace(shortcut, symbol);
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
        let mut source = "let angle = \\theta; print(\\pi); \\[ angle \\]".to_owned();
        expand_greek_shortcuts(&mut source);
        assert_eq!(source, "let angle = θ; print(π); \\[ angle \\]");
    }

    #[test]
    fn highlights_lexer_tokens_and_distinguishes_render_markers() {
        let theme = Theme::gruber_darker();
        let source = "let x = 42; \\[ x =>; \\]";
        let job = highlight(source, &theme, FontId::monospace(15.0), &[]);
        assert_eq!(color_at(&job, "let"), theme.syntax.keyword.0);
        assert_eq!(color_at(&job, "42"), theme.syntax.number.0);
        assert_eq!(color_at(&job, "\\["), theme.syntax.render_marker.0);
        assert_ne!(color_at(&job, "\\["), theme.syntax.delimiter.0);
    }

    #[test]
    fn highlights_doc_and_ordinary_comments_without_coloring_slashes_in_strings() {
        let theme = Theme::gruber_darker();
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
            "Quadratic formula\n\n\\[\\frac{\\left(\\mathrm{x} + 1\\right)}{\\left(2\\mathrm{x}^{2}\\right)}\\]"
        );
        assert_eq!(
            readable_latex(&preview),
            "Quadratic formula\n\n((x + 1))⁄((2x²))"
        );
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
            &Theme::gruber_darker(),
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
        let rendered = render_latex_png(r"\[\frac{x^2}{2}\]", [20, 20, 20]).unwrap();
        assert!(rendered.size[0] > 20);
        assert!(rendered.size[1] > 20);
        assert_eq!(rendered.rgba.len(), rendered.size[0] * rendered.size[1] * 4);
    }
}
