//! Browser entry point for a real egui SCAS workspace. The native IDE remains
//! separate while platform services are being extracted from its binary.

#[path = "bin/scas_gui/math_preview.rs"]
mod math_preview;

use crate::ast::SourceSpan;
use crate::document::{Document, DocumentFragment, FragmentKind};
use crate::web_runtime::{MIT_EXAMPLE, STARTER};
use eframe::egui::{self, Color32, FontId, RichText, TextEdit};
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
                Box::new(|_cc| Ok(Box::new(WebApp::new()))),
            )
            .await
    }

    pub fn load_source(&self, source: String) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.load_source(source);
        }
    }
}

impl Default for WebHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Document,
    Plot,
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
}

impl WebApp {
    fn new() -> Self {
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

impl eframe::App for WebApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        context.set_visuals(egui::Visuals::dark());
        self.poll(context);
        let now = context.input(|input| input.time);
        if self.due.is_some_and(|due| now >= due) {
            self.due = None;
            self.request("run");
        }
        let dark = Color32::from_rgb(31, 31, 31);
        let panel = Color32::from_rgb(37, 37, 38);
        let muted = Color32::from_rgb(157, 157, 157);
        let text = Color32::from_rgb(204, 204, 204);
        let mut jump_to = None;
        egui::TopBottomPanel::top("web-toolbar")
            .frame(egui::Frame::none().fill(panel))
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("SCAS")
                            .strong()
                            .color(Color32::from_rgb(79, 156, 249))
                            .size(15.0),
                    );
                    ui.separator();
                    if ui
                        .selectable_label(self.tab == Tab::Document, "Document")
                        .clicked()
                    {
                        self.tab = Tab::Document;
                    }
                    if ui
                        .selectable_label(self.tab == Tab::Plot, "Graphing")
                        .clicked()
                    {
                        self.tab = Tab::Plot;
                    }
                    ui.separator();
                    if ui.button("Run").clicked() {
                        self.request("run");
                    }
                    ui.hyperlink_to("Docs", "./docs/index.html");
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
                        if ui.button("Import .scas").clicked() {
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
                                let hash = format!(
                                    "source={}",
                                    js_sys::encode_uri_component(&self.source)
                                );
                                let _ = window.location().set_hash(&hash);
                                if let Ok(url) = window.location().href() {
                                    ui.output_mut(|output| output.copied_text = url);
                                }
                                "Share link updated and copied".clone_into(&mut self.status);
                            }
                            ui.close_menu();
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("web-status")
            .frame(egui::Frame::none().fill(Color32::from_rgb(0, 122, 204)))
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.status).color(Color32::WHITE).size(11.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("SCAS · Browser")
                                .color(Color32::WHITE)
                                .size(11.0),
                        );
                    });
                });
            });
        if self.tab == Tab::Document {
            egui::TopBottomPanel::bottom("web-output")
                .resizable(true)
                .default_height(150.0)
                .height_range(120.0..=360.0)
                .frame(egui::Frame::none().fill(dark))
                .show(context, |ui| {
                    ui.label(RichText::new("OUTPUT / PROBLEMS").strong().size(11.0));
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for problem in &self.problems {
                            let label = format!(
                                "browser.scas:{}:{}  {}",
                                problem.line, problem.column, problem.message
                            );
                            if ui
                                .selectable_label(
                                    false,
                                    RichText::new(label).color(Color32::from_rgb(249, 119, 127)),
                                )
                                .clicked()
                            {
                                jump_to = Some((problem.line, problem.column));
                            }
                        }
                        if let Some(error) = &self.error {
                            ui.colored_label(Color32::from_rgb(249, 119, 127), error);
                        }
                        if !self.output.is_empty() {
                            ui.monospace(&self.output);
                        }
                        if self.error.is_none() && self.output.is_empty() {
                            ui.label(RichText::new("No output yet.").color(muted));
                        }
                    });
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
            let narrow = context.screen_rect().width() < 850.0;
            if !narrow {
                egui::SidePanel::right("web-preview")
                    .resizable(true)
                    .default_width(500.0)
                    .min_width(260.0)
                    .frame(egui::Frame::none().fill(panel))
                    .show(context, |ui| {
                        ui.label(RichText::new("DOCUMENT PREVIEW").strong().size(11.0));
                        ui.separator();
                        egui::ScrollArea::both().show(ui, |ui| {
                            if let Some(document) = &self.document {
                                math_preview::show_document(ui, document, text, muted, 1.0, None);
                            } else {
                                ui.label("Run a document to preview its mathematics.");
                            }
                        });
                        if ui.button("Download .tex").clicked() {
                            download_file("document.tex", &self.tex, "text/x-tex");
                        }
                    });
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(dark))
                .show(context, |ui| {
                    ui.label(RichText::new(&self.source_name).size(12.0).strong());
                    ui.separator();
                    let available_rows = if narrow {
                        (ui.available_height() * 0.48 / 22.0) as usize
                    } else {
                        (ui.available_height() / 22.0) as usize
                    };
                    let response = egui::ScrollArea::vertical()
                        .show(ui, |ui| {
                            TextEdit::multiline(&mut self.source)
                                .id(egui::Id::new("web-source"))
                                .font(FontId::monospace(17.0))
                                .code_editor()
                                .desired_width(ui.available_width())
                                .desired_rows(available_rows.max(12))
                                .show(ui)
                                .response
                        })
                        .inner;
                    if response.changed() {
                        self.save_draft();
                        self.due = Some(now + 0.4);
                        context.request_repaint_after(Duration::from_millis(420));
                    }
                    if narrow {
                        ui.separator();
                        ui.label(RichText::new("DOCUMENT PREVIEW").strong().size(11.0));
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                if let Some(document) = &self.document {
                                    math_preview::show_document(
                                        ui, document, text, muted, 0.85, None,
                                    );
                                }
                            });
                    }
                });
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
    }
}
