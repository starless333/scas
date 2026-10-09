//! Browser entry point for a real egui SCAS workspace. The native IDE remains
//! separate while platform services are being extracted from its binary.

use crate::ast::SourceSpan;
use crate::document::{Document, DocumentFragment, FragmentKind};
use crate::lexer::{Scanner, TokenType};
use crate::web_runtime::{MIT_EXAMPLE, STARTER};
use eframe::egui::{
    self,
    text::{LayoutJob, TextFormat},
    Color32, FontData, FontDefinitions, FontFamily, FontId, RichText, TextEdit,
};
use serde_json::{json, Value};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, Worker};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = scasDownload)]
    fn download_file(name: &str, content: &str, mime: &str);
    #[wasm_bindgen(js_namespace = window, js_name = scasImport)]
    fn import_file();
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

    pub fn close_preview(&self) -> bool {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.preview_open = false;
            app.context.request_repaint();
            true
        } else {
            false
        }
    }
}

impl Default for WebHandle {
    fn default() -> Self {
        Self::new()
    }
}

fn configure_context(context: &egui::Context) {
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
    context.set_fonts(fonts);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(37, 37, 38);
    visuals.window_fill = Color32::from_rgb(37, 37, 38);
    visuals.extreme_bg_color = Color32::from_rgb(31, 31, 31);
    visuals.faint_bg_color = Color32::from_rgb(31, 31, 31);
    visuals.override_text_color = Some(Color32::from_rgb(204, 204, 204));
    visuals.selection.bg_fill = Color32::from_rgb(38, 79, 120);
    visuals.widgets.active.bg_fill = Color32::from_rgb(0, 122, 204);
    context.set_visuals(visuals);
    let mut style = (*context.style()).clone();
    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(12.0));
    context.set_style(style);
}

fn highlight_source(source: &str, font: FontId) -> LayoutJob {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    let mut spans = tokens
        .iter()
        .filter_map(|token| {
            use TokenType::*;
            let color = match token.token_type {
                Eof => return None,
                Let | Fn | If | Else | For | While | Break | Continue | Return | Print | And
                | Or | Xor | True | False | Nil | Exit | Help | Clear => {
                    Color32::from_rgb(197, 134, 192)
                }
                Number => Color32::from_rgb(181, 206, 168),
                Str => Color32::from_rgb(206, 145, 120),
                DocComment => Color32::from_rgb(96, 139, 78),
                Identifier => Color32::from_rgb(156, 220, 254),
                RenderLeftBrace | RenderRightBrace | RenderLeftBracket | RenderRightBracket => {
                    Color32::from_rgb(78, 201, 176)
                }
                LeftParen | RightParen | LeftBracket | RightBracket | LeftBrace | RightBrace
                | Comma | Dot | Colon | Semicolon => Color32::from_rgb(255, 215, 0),
                _ => Color32::from_rgb(212, 212, 212),
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
                    Color32::from_rgb(106, 153, 85),
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
        append(&source[cursor..start], Color32::from_rgb(212, 212, 212));
        append(&source[start..end], color);
        cursor = end;
    }
    append(&source[cursor..], Color32::from_rgb(212, 212, 212));
    job
}

fn paint_line_numbers(ui: &egui::Ui, output: &egui::text_edit::TextEditOutput, gutter: f32) {
    let left = output.galley_pos.x - gutter;
    let clip = egui::Rect::from_min_max(
        egui::pos2(left, output.response.rect.top()),
        output.response.rect.right_bottom(),
    )
    .intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(clip);
    let separator = output.galley_pos.x - 10.0;
    painter.line_segment(
        [
            egui::pos2(separator, clip.top()),
            egui::pos2(separator, clip.bottom()),
        ],
        egui::Stroke::new(1.0, Color32::from_rgb(53, 53, 53)),
    );
    let mut line = 1;
    let mut first_row = true;
    for row in &output.galley.rows {
        if first_row {
            painter.text(
                egui::pos2(separator - 10.0, output.galley_pos.y + row.rect.center().y),
                egui::Align2::RIGHT_CENTER,
                line.to_string(),
                FontId::monospace(18.0),
                Color32::from_rgb(0, 122, 204),
            );
        }
        first_row = row.ends_with_newline;
        if row.ends_with_newline {
            line += 1;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Document,
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
    output_open: bool,
    bottom_tab: BottomTab,
    highlight_source: String,
    highlight_job: LayoutJob,
    first_frame: bool,
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
        let shared = web_sys::window()
            .and_then(|window| window.location().hash().ok())
            .and_then(|hash| hash.strip_prefix("#source=").map(str::to_owned))
            .and_then(|encoded| js_sys::decode_uri_component(&encoded).ok())
            .map(String::from);
        let source = shared.or(stored).unwrap_or_else(|| STARTER.to_owned());
        let mut app = Self {
            context,
            source,
            output: String::new(),
            error: None,
            problems: Vec::new(),
            document: None,
            tex: String::new(),
            tab: Tab::Document,
            plot_expression: "sin(x)".to_owned(),
            plot_series: Vec::new(),
            plot_bounds: [-10.0, 10.0, -1.0, 1.0],
            plot_svg: String::new(),
            generation: 0,
            pending: false,
            due: None,
            worker,
            _onmessage: onmessage,
            responses,
            status,
            source_name: "untitled.scas".to_owned(),
            preview_json: "[]".to_owned(),
            explorer_open: true,
            preview_open: true,
            output_open: false,
            bottom_tab: BottomTab::Output,
            highlight_source: String::new(),
            highlight_job: LayoutJob::default(),
            first_frame: true,
        };
        app.request("run");
        app
    }

    fn load_source(&mut self, source: String) {
        self.source = source;
        "imported.scas".clone_into(&mut self.source_name);
        self.save_draft();
        self.request("run");
    }

    fn save_draft(&self) {
        if let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        {
            let _ = storage.set_item("scas.draft", &self.source);
        }
    }

    fn request(&mut self, kind: &str) {
        self.generation = self.generation.wrapping_add(1);
        let request = json!({
            "generation": self.generation,
            "kind": kind,
            "source": self.source,
            "expression": self.plot_expression,
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
                        self.plot_bounds[index] = bound.as_f64().unwrap_or(self.plot_bounds[index]);
                    }
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

    fn plot(&self, ui: &mut egui::Ui) {
        if self.plot_series.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Enter an expression, then select Plot.");
            });
            return;
        }
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(31, 31, 31));
        let [x_min, x_max, y_min, y_max] = self.plot_bounds;
        let map = |x: f64, y: f64| {
            egui::pos2(
                rect.left() + ((x - x_min) / (x_max - x_min)) as f32 * rect.width(),
                rect.bottom() - ((y - y_min) / (y_max - y_min)) as f32 * rect.height(),
            )
        };
        let grid = Color32::from_gray(62);
        if x_min <= 0.0 && 0.0 <= x_max {
            let x = map(0.0, 0.0).x;
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(1.0, grid),
            );
        }
        if y_min <= 0.0 && 0.0 <= y_max {
            let y = map(0.0, 0.0).y;
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(1.0, grid),
            );
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
    }
}

impl WebApp {
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
        );
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            ui.menu_button("Examples", |ui| {
                if ui.button("Starter document").clicked() {
                    self.load_source(STARTER.to_owned());
                    "starter.scas".clone_into(&mut self.source_name);
                    ui.close_menu();
                }
                if ui.button("MIT 18.06 · Diagonalization").clicked() {
                    self.load_source(MIT_EXAMPLE.to_owned());
                    "mit-18-06-session-22.scas".clone_into(&mut self.source_name);
                    ui.close_menu();
                }
            });
            ui.separator();
            if ui.button("Import .scas…").clicked() {
                import_file();
                ui.close_menu();
            }
            if ui.button("Download .scas").clicked() {
                download_file(&self.source_name, &self.source, "text/plain");
                ui.close_menu();
            }
            if ui.button("Download .tex").clicked() {
                download_file("document.tex", &self.tex, "text/x-tex");
                ui.close_menu();
            }
            if ui.button("Share link").clicked() {
                if let Some(window) = web_sys::window() {
                    let hash = format!("source={}", js_sys::encode_uri_component(&self.source));
                    let _ = window.location().set_hash(&hash);
                    if let Ok(url) = window.location().href() {
                        ui.output_mut(|output| output.copied_text = url);
                    }
                    "Share link updated and copied".clone_into(&mut self.status);
                }
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
            if context.screen_rect().width() < 850.0 {
                self.explorer_open = false;
                self.preview_open = false;
            }
            self.first_frame = false;
        }
        let editor_font = FontId::monospace(24.0);
        if self.highlight_source != self.source {
            self.highlight_job = highlight_source(&self.source, editor_font.clone());
            self.highlight_source.clone_from(&self.source);
        }
        let now = context.input(|input| input.time);
        if self.due.is_some_and(|due| now >= due) {
            self.due = None;
            self.request("run");
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::B)) {
            self.explorer_open = !self.explorer_open;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::J)) {
            self.output_open = !self.output_open;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)) {
            self.request("run");
            self.output_open = true;
        }
        let dark = Color32::from_rgb(31, 31, 31);
        let panel = Color32::from_rgb(37, 37, 38);
        let muted = Color32::from_rgb(157, 157, 157);
        let border = egui::Stroke::new(1.0, Color32::from_rgb(58, 58, 59));
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
                    .fill(Color32::from_rgb(60, 60, 60))
                    .inner_margin(egui::Margin::symmetric(8.0, 4.0)),
            )
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        RichText::new("SCAS")
                            .strong()
                            .size(15.0)
                            .color(Color32::from_rgb(224, 224, 224)),
                    );
                    ui.add_space(12.0);
                    self.file_menu(ui);
                    ui.menu_button("View", |ui| {
                        ui.checkbox(&mut self.explorer_open, "Explorer  Ctrl+B");
                        ui.checkbox(&mut self.preview_open, "Document Preview");
                        ui.checkbox(&mut self.output_open, "Output Panel  Ctrl+J");
                        if ui.button("Problems").clicked() {
                            self.bottom_tab = BottomTab::Problems;
                            self.output_open = true;
                            ui.close_menu();
                        }
                    });
                    if !narrow {
                        ui.add_space(26.0);
                        egui::Frame::none()
                            .fill(Color32::from_rgb(50, 50, 50))
                            .stroke(border)
                            .inner_margin(egui::Margin::symmetric(15.0, 3.0))
                            .show(ui, |ui| {
                                ui.label(RichText::new(&self.source_name).size(12.0));
                            });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.hyperlink_to("Docs", "./docs/index.html");
                        if ui
                            .button(RichText::new('\u{eb2c}').font(FontId::new(
                                16.0,
                                FontFamily::Name("VS Code Codicons".into()),
                            )))
                            .on_hover_text("Run document · Ctrl+Enter")
                            .clicked()
                        {
                            self.request("run");
                            self.output_open = true;
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("web-status")
            .exact_height(26.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0, 122, 204))
                    .inner_margin(egui::Margin::symmetric(8.0, 2.0)),
            )
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    if ui.button(format!("× {}", self.problems.len())).clicked() {
                        self.bottom_tab = BottomTab::Problems;
                        self.output_open = true;
                    }
                    ui.label(RichText::new(&self.status).color(Color32::WHITE).size(11.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("UTF-8    { } SCAS")
                                .color(Color32::WHITE)
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
                    .fill(Color32::from_rgb(51, 51, 51))
                    .stroke(border)
                    .inner_margin(egui::Margin::symmetric(3.0, 6.0)),
            )
            .show(context, |ui| {
                ui.vertical_centered(|ui| {
                    if ui
                        .selectable_label(
                            self.explorer_open,
                            RichText::new('\u{eaf0}').font(FontId::new(
                                22.0,
                                FontFamily::Name("VS Code Codicons".into()),
                            )),
                        )
                        .on_hover_text("Explorer · Ctrl+B")
                        .clicked()
                    {
                        self.explorer_open = !self.explorer_open;
                    }
                    if ui
                        .selectable_label(
                            self.tab == Tab::Document,
                            RichText::new('\u{ea7b}').font(FontId::new(
                                22.0,
                                FontFamily::Name("VS Code Codicons".into()),
                            )),
                        )
                        .on_hover_text("Document")
                        .clicked()
                    {
                        self.tab = Tab::Document;
                    }
                    if ui
                        .selectable_label(
                            self.tab == Tab::Plot,
                            RichText::new('\u{ebe2}').font(FontId::new(
                                22.0,
                                FontFamily::Name("VS Code Codicons".into()),
                            )),
                        )
                        .on_hover_text("Graphing")
                        .clicked()
                    {
                        self.tab = Tab::Plot;
                    }
                });
            });
        if self.explorer_open {
            egui::SidePanel::left("web-explorer")
                .default_width(260.0).width_range(180.0..=400.0).resizable(true)
                .frame(egui::Frame::none().fill(panel).stroke(border)
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)))
                .show(context, |ui| {
                    ui.label(RichText::new("Explorer").strong().size(13.0));
                    ui.add_space(15.0);
                    ui.label(RichText::new("OPEN EDITORS").strong().size(11.0).color(muted));
                    if ui.selectable_label(self.tab == Tab::Document, format!("  SC   {}", self.source_name)).clicked() {
                        self.tab = Tab::Document;
                    }
                    ui.add_space(16.0);
                    ui.label(RichText::new("BROWSER WORKSPACE").strong().size(11.0).color(muted));
                    if ui.selectable_label(self.tab == Tab::Plot, "  ⌁   Graphing").clicked() {
                        self.tab = Tab::Plot;
                    }
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        ui.label(RichText::new("Files are kept in this browser. Use File to import or download a .scas document.")
                            .size(11.0).color(muted));
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
                    egui::SidePanel::right("web-preview")
                        .resizable(true)
                        .default_width(480.0)
                        .width_range(280.0..=750.0)
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
                        ui.horizontal(|ui| {
                            ui.set_min_height(30.0);
                            ui.label(
                                RichText::new("SC")
                                    .strong()
                                    .size(12.0)
                                    .color(Color32::from_rgb(0, 122, 204)),
                            );
                            ui.label(RichText::new(&self.source_name).size(12.0));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button(RichText::new('\u{ebf4}').font(FontId::new(
                                            16.0,
                                            FontFamily::Name("VS Code Codicons".into()),
                                        )))
                                        .on_hover_text("Toggle mathematical preview")
                                        .clicked()
                                    {
                                        self.preview_open = !self.preview_open;
                                    }
                                },
                            );
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&self.source_name).size(11.0).color(muted));
                        ui.label(RichText::new("›").size(11.0).color(muted));
                        ui.label(RichText::new("SCAS").size(11.0).color(muted));
                    });
                    ui.add_space(4.0);
                    let available_rows = (ui.available_height() / 24.0) as usize;
                    let cached_source = &self.highlight_source;
                    let cached_job = &self.highlight_job;
                    let mut layouter = |ui: &egui::Ui, source: &str, width: f32| {
                        let mut job = if source == cached_source {
                            cached_job.clone()
                        } else {
                            highlight_source(source, editor_font.clone())
                        };
                        job.wrap.max_width = width;
                        ui.fonts(|fonts| fonts.layout_job(job))
                    };
                    let gutter = 62.0;
                    let response = egui::ScrollArea::both()
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
                            paint_line_numbers(ui, &output, gutter);
                            output.response
                        })
                        .inner;
                    if response.changed() {
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
        } else {
            egui::TopBottomPanel::top("plot-controls")
                .frame(egui::Frame::none().fill(panel))
                .show(context, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("y =");
                        ui.text_edit_singleline(&mut self.plot_expression);
                        if ui.button("Plot").clicked() {
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
                .show(context, |ui| self.plot(ui));
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
            );
        }
        if !preview_visible {
            hide_preview();
        }
    }
}
