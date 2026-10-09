//! A PTY-backed terminal screen. The shell receives keystrokes as they happen;
//! VT output is parsed into cells, including the alternate screen used by TUIs.

use eframe::egui::{self, Color32, FontId, TextFormat};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};

pub const TERMINAL_ID: &str = "terminal-screen";
const FONT_SIZE: f32 = 13.0;
const SCROLLBACK_ROWS: usize = 10_000;

pub struct TerminalSession {
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    rows: u16,
    cols: u16,
}

impl TerminalSession {
    pub fn start(directory: &Path) -> Result<Self, String> {
        let rows = 24;
        let cols = 80;
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;
        let shell = std::env::var_os("SHELL")
            .or_else(|| std::env::var_os("COMSPEC"))
            .unwrap_or_else(|| if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }.into());
        let mut command = CommandBuilder::new(shell);
        #[cfg(unix)]
        command.arg("-i");
        command.cwd(directory);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        drop(pair.slave);
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 || sender.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            master: pair.master,
            writer,
            output,
            parser: vt100::Parser::new(rows, cols, SCROLLBACK_ROWS),
            rows,
            cols,
        })
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(bytes) = self.output.try_recv() {
            self.parser.process(&bytes);
            changed = true;
        }
        changed
    }

    pub fn clear(&mut self) {
        self.parser.process(b"\x1b[2J\x1b[H");
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.writer
            .write_all(bytes)
            .map_err(|error| error.to_string())?;
        self.writer.flush().map_err(|error| error.to_string())
    }

    fn resize(&mut self, rows: u16, cols: u16, width: f32, height: f32) -> Result<(), String> {
        if (rows, cols) == (self.rows, self.cols) {
            return Ok(());
        }
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: width.max(0.0).min(u16::MAX as f32) as u16,
                pixel_height: height.max(0.0).min(u16::MAX as f32) as u16,
            })
            .map_err(|error| error.to_string())?;
        self.parser.set_size(rows, cols);
        self.rows = rows;
        self.cols = cols;
        Ok(())
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        foreground: Color32,
        background: Color32,
    ) -> Result<(), String> {
        let size = ui.available_size();
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let response = ui.interact(rect, egui::Id::new(TERMINAL_ID), egui::Sense::click());
        if response.clicked() {
            response.request_focus();
        }
        let font = FontId::monospace(FONT_SIZE);
        let (cell_width, cell_height) =
            ui.fonts(|fonts| (fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));
        let cols = ((rect.width() / cell_width).floor() as u16).max(2);
        let rows = ((rect.height() / cell_height).floor() as u16).max(2);
        self.resize(rows, cols, rect.width(), rect.height())?;
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, background);

        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll.abs() > 1.0 {
                if self.parser.screen().mouse_protocol_mode() == vt100::MouseProtocolMode::None {
                    let current = self.parser.screen().scrollback() as i32;
                    let amount = (scroll / cell_height).round() as i32;
                    self.parser
                        .set_scrollback(current.saturating_add(amount).max(0) as usize);
                } else if let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) {
                    self.send_mouse(
                        rect,
                        cell_width,
                        cell_height,
                        pointer,
                        if scroll > 0.0 { 64 } else { 65 },
                        true,
                    )?;
                }
            }
        }

        let screen = self.parser.screen();
        for row in 0..rows {
            let mut job = egui::text::LayoutJob::default();
            job.wrap.max_width = f32::INFINITY;
            let mut skip_next = false;
            for col in 0..cols {
                if skip_next {
                    skip_next = false;
                    continue;
                }
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                let inverse = cell.inverse();
                let mut format = TextFormat::simple(
                    font.clone(),
                    terminal_color(
                        if inverse {
                            cell.bgcolor()
                        } else {
                            cell.fgcolor()
                        },
                        foreground,
                        false,
                    ),
                );
                format.background = terminal_color(
                    if inverse {
                        cell.fgcolor()
                    } else {
                        cell.bgcolor()
                    },
                    background,
                    true,
                );
                if cell.underline() {
                    format.underline = egui::Stroke::new(1.0, format.color);
                }
                let contents = cell.contents();
                job.append(
                    if contents.is_empty() { " " } else { &contents },
                    0.0,
                    format,
                );
                skip_next = cell.is_wide();
            }
            let galley = ui.fonts(|fonts| fonts.layout_job(job));
            painter.galley(
                rect.min + egui::vec2(0.0, row as f32 * cell_height),
                galley,
                foreground,
            );
        }
        if !screen.hide_cursor() && screen.scrollback() == 0 {
            let (row, col) = screen.cursor_position();
            let cursor = egui::Rect::from_min_size(
                rect.min + egui::vec2(col as f32 * cell_width, row as f32 * cell_height),
                egui::vec2(cell_width.max(2.0), cell_height),
            );
            painter.rect_stroke(cursor, 0.0, egui::Stroke::new(1.4, foreground));
        }

        if response.has_focus() {
            for event in ui.input(|input| input.events.clone()) {
                match event {
                    egui::Event::Text(text) => {
                        if ui.input(|input| input.modifiers.alt) {
                            self.write(b"\x1b")?;
                        }
                        self.write(text.as_bytes())?;
                    }
                    egui::Event::Paste(text) => {
                        if self.parser.screen().bracketed_paste() {
                            self.write(b"\x1b[200~")?;
                            self.write(text.as_bytes())?;
                            self.write(b"\x1b[201~")?;
                        } else {
                            self.write(text.as_bytes())?;
                        }
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if let Some(bytes) =
                            key_bytes(key, modifiers, self.parser.screen().application_cursor())
                        {
                            self.write(&bytes)?;
                        }
                    }
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        ..
                    } if rect.contains(pos)
                        && self.parser.screen().mouse_protocol_mode()
                            != vt100::MouseProtocolMode::None =>
                    {
                        let code = match button {
                            egui::PointerButton::Primary => 0,
                            egui::PointerButton::Middle => 1,
                            egui::PointerButton::Secondary => 2,
                            _ => 3,
                        };
                        self.send_mouse(rect, cell_width, cell_height, pos, code, pressed)?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    fn send_mouse(
        &mut self,
        rect: egui::Rect,
        cell_width: f32,
        cell_height: f32,
        pos: egui::Pos2,
        button: u8,
        pressed: bool,
    ) -> Result<(), String> {
        let col = (((pos.x - rect.left()) / cell_width).floor() as u16 + 1).clamp(1, self.cols);
        let row = (((pos.y - rect.top()) / cell_height).floor() as u16 + 1).clamp(1, self.rows);
        let bytes = if self.parser.screen().mouse_protocol_encoding()
            == vt100::MouseProtocolEncoding::Sgr
        {
            format!(
                "\x1b[<{button};{col};{row}{}",
                if pressed { 'M' } else { 'm' }
            )
            .into_bytes()
        } else {
            let code = if pressed { button } else { 3 };
            vec![
                0x1b,
                b'[',
                b'M',
                32 + code,
                (32 + col.min(223)) as u8,
                (32 + row.min(223)) as u8,
            ]
        };
        self.write(&bytes)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn key_bytes(
    key: egui::Key,
    modifiers: egui::Modifiers,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    use egui::Key;
    let bytes: &[u8] = if modifiers.ctrl {
        match key {
            Key::A => b"\x01",
            Key::B => b"\x02",
            Key::C => b"\x03",
            Key::D => b"\x04",
            Key::E => b"\x05",
            Key::F => b"\x06",
            Key::G => b"\x07",
            Key::H => b"\x08",
            Key::I => b"\x09",
            Key::J => b"\x0a",
            Key::K => b"\x0b",
            Key::L => b"\x0c",
            Key::M => b"\x0d",
            Key::N => b"\x0e",
            Key::O => b"\x0f",
            Key::P => b"\x10",
            Key::Q => b"\x11",
            Key::R => b"\x12",
            Key::S => b"\x13",
            Key::T => b"\x14",
            Key::U => b"\x15",
            Key::V => b"\x16",
            Key::W => b"\x17",
            Key::X => b"\x18",
            Key::Y => b"\x19",
            Key::Z => b"\x1a",
            _ => b"",
        }
    } else {
        match key {
            Key::Enter => b"\r",
            Key::Tab if modifiers.shift => b"\x1b[Z",
            Key::Tab => b"\t",
            Key::Backspace => b"\x7f",
            Key::Escape => b"\x1b",
            Key::ArrowUp => {
                if application_cursor {
                    b"\x1bOA"
                } else {
                    b"\x1b[A"
                }
            }
            Key::ArrowDown => {
                if application_cursor {
                    b"\x1bOB"
                } else {
                    b"\x1b[B"
                }
            }
            Key::ArrowRight => {
                if application_cursor {
                    b"\x1bOC"
                } else {
                    b"\x1b[C"
                }
            }
            Key::ArrowLeft => {
                if application_cursor {
                    b"\x1bOD"
                } else {
                    b"\x1b[D"
                }
            }
            Key::Home => b"\x1b[H",
            Key::End => b"\x1b[F",
            Key::Delete => b"\x1b[3~",
            Key::Insert => b"\x1b[2~",
            Key::PageUp => b"\x1b[5~",
            Key::PageDown => b"\x1b[6~",
            Key::F1 => b"\x1bOP",
            Key::F2 => b"\x1bOQ",
            Key::F3 => b"\x1bOR",
            Key::F4 => b"\x1bOS",
            Key::F5 => b"\x1b[15~",
            Key::F6 => b"\x1b[17~",
            Key::F7 => b"\x1b[18~",
            Key::F8 => b"\x1b[19~",
            Key::F9 => b"\x1b[20~",
            Key::F10 => b"\x1b[21~",
            Key::F11 => b"\x1b[23~",
            Key::F12 => b"\x1b[24~",
            _ => b"",
        }
    };
    if bytes.is_empty() {
        None
    } else if modifiers.alt {
        let mut result = vec![0x1b];
        result.extend_from_slice(bytes);
        Some(result)
    } else {
        Some(bytes.to_vec())
    }
}

fn terminal_color(color: vt100::Color, fallback: Color32, background: bool) -> Color32 {
    match color {
        vt100::Color::Default => fallback,
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        vt100::Color::Idx(index) => {
            const ANSI: [[u8; 3]; 16] = [
                [0, 0, 0],
                [205, 49, 49],
                [13, 188, 121],
                [229, 229, 16],
                [36, 114, 200],
                [188, 63, 188],
                [17, 168, 205],
                [229, 229, 229],
                [102, 102, 102],
                [241, 76, 76],
                [35, 209, 139],
                [245, 245, 67],
                [59, 142, 234],
                [214, 112, 214],
                [41, 184, 219],
                [255, 255, 255],
            ];
            let rgb = if index < 16 {
                ANSI[index as usize]
            } else if index < 232 {
                let offset = index - 16;
                let level = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
                [
                    level(offset / 36),
                    level((offset / 6) % 6),
                    level(offset % 6),
                ]
            } else {
                let gray = 8 + (index - 232) * 10;
                [gray, gray, gray]
            };
            let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            if background && index == 0 {
                fallback
            } else {
                color
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vt_screen_tracks_colors_cursor_and_alternate_buffer() {
        let mut parser = vt100::Parser::new(4, 20, 20);
        parser.process(b"ready\x1b[31m RED\x1b[0m");
        assert!(parser.screen().contents().contains("ready RED"));
        assert_eq!(
            parser.screen().cell(0, 6).unwrap().fgcolor(),
            vt100::Color::Idx(1)
        );
        parser.process(b"\x1b[?1049hfull screen");
        assert!(parser.screen().alternate_screen());
        assert!(parser.screen().contents().contains("full screen"));
        parser.process(b"\x1b[?1049l");
        assert!(parser.screen().contents().contains("ready RED"));
    }

    #[test]
    fn shell_accepts_direct_keystrokes_and_resizes() {
        let mut session = TerminalSession::start(Path::new(".")).unwrap();
        session.resize(35, 91, 900.0, 500.0).unwrap();
        assert_eq!(session.master.get_size().unwrap().cols, 91);
        session.write(b"printf '%s\\n' $((6 * 7))\r").unwrap();
        for _ in 0..100 {
            session.poll();
            if session.screen().contents().contains("42") {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("shell did not render its command output");
    }
}
