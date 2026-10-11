//! Host services used by SCAS programs. Native execution uses real files and
//! standard input; browser execution uses an in-memory filesystem and queued
//! prompt answers supplied by the UI.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoFailure {
    Message(String),
    InputRequired(String),
}

impl std::fmt::Display for IoFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => message.fmt(formatter),
            Self::InputRequired(prompt) => write!(formatter, "input required: {prompt}"),
        }
    }
}

pub trait RuntimeIo {
    fn read_text(&self, path: &str) -> Result<String, IoFailure>;
    fn write_text(&self, path: &str, content: &str) -> Result<(), IoFailure>;
    fn input(&self, prompt: &str) -> Result<String, IoFailure>;
    fn args(&self) -> Vec<String>;
}

#[derive(Default)]
pub struct NativeIo;

#[cfg(not(target_arch = "wasm32"))]
impl RuntimeIo for NativeIo {
    fn read_text(&self, path: &str) -> Result<String, IoFailure> {
        std::fs::read_to_string(path)
            .map_err(|error| IoFailure::Message(format!("could not read {path}: {error}")))
    }

    fn write_text(&self, path: &str, content: &str) -> Result<(), IoFailure> {
        std::fs::write(path, content)
            .map_err(|error| IoFailure::Message(format!("could not write {path}: {error}")))
    }

    fn input(&self, prompt: &str) -> Result<String, IoFailure> {
        use std::io::{self, Write};
        eprint!("{prompt}");
        io::stderr()
            .flush()
            .map_err(|error| IoFailure::Message(error.to_string()))?;
        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .map_err(|error| IoFailure::Message(error.to_string()))?;
        Ok(line.trim_end_matches(['\r', '\n']).to_owned())
    }

    fn args(&self) -> Vec<String> {
        std::env::args().skip(1).collect()
    }
}

/// Virtual files are held in insertion-independent path order. Only relative
/// paths without parent traversal are accepted, matching browser uploads.
#[derive(Default)]
pub struct MemoryIo {
    files: RefCell<BTreeMap<String, String>>,
    inputs: RefCell<VecDeque<String>>,
    arguments: RefCell<Vec<String>>,
}

impl MemoryIo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn normalize(path: &str) -> Result<String, IoFailure> {
        let normalized = path.replace('\\', "/");
        if normalized.starts_with('/') {
            return Err(IoFailure::Message(
                "virtual paths must be relative".to_owned(),
            ));
        }
        let parts = normalized
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect::<Vec<_>>();
        if parts.is_empty() || parts.iter().any(|part| *part == "..") {
            return Err(IoFailure::Message(
                "virtual path is empty or escapes the workspace".to_owned(),
            ));
        }
        Ok(parts.join("/"))
    }

    pub fn insert_file(&self, path: &str, content: String) -> Result<(), IoFailure> {
        let path = Self::normalize(path)?;
        self.files.borrow_mut().insert(path, content);
        Ok(())
    }

    pub fn queue_input(&self, value: String) {
        self.inputs.borrow_mut().push_back(value);
    }

    pub fn clear_inputs(&self) {
        self.inputs.borrow_mut().clear();
    }

    pub fn set_args(&self, values: Vec<String>) {
        *self.arguments.borrow_mut() = values;
    }

    pub fn file_names(&self) -> Vec<String> {
        self.files.borrow().keys().cloned().collect()
    }
}

impl RuntimeIo for MemoryIo {
    fn read_text(&self, path: &str) -> Result<String, IoFailure> {
        let path = Self::normalize(path)?;
        self.files
            .borrow()
            .get(&path)
            .cloned()
            .ok_or_else(|| IoFailure::Message(format!("file '{path}' was not found")))
    }

    fn write_text(&self, path: &str, content: &str) -> Result<(), IoFailure> {
        self.insert_file(path, content.to_owned())
    }

    fn input(&self, prompt: &str) -> Result<String, IoFailure> {
        self.inputs
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| IoFailure::InputRequired(prompt.to_owned()))
    }

    fn args(&self) -> Vec<String> {
        self.arguments.borrow().clone()
    }
}

thread_local! {
    static BROWSER_IO: Rc<MemoryIo> = {
        let io = Rc::new(MemoryIo::new());
        io.insert_file("stdlib/core.scas", include_str!("../stdlib/core.scas").to_owned()).expect("bundled standard library path is valid");
        io
    };
}

pub fn browser_io() -> Rc<MemoryIo> {
    BROWSER_IO.with(Rc::clone)
}

pub fn default_io() -> Rc<dyn RuntimeIo> {
    #[cfg(target_arch = "wasm32")]
    {
        browser_io()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Rc::new(NativeIo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_io_preserves_files_and_rejects_traversal() {
        let io = MemoryIo::new();
        io.insert_file("data/values.csv", "x,y\n1,2".to_owned())
            .unwrap();
        assert_eq!(io.read_text("data/./values.csv").unwrap(), "x,y\n1,2");
        assert!(io.write_text("../outside", "no").is_err());
        io.queue_input("42".to_owned());
        assert_eq!(io.input("number? ").unwrap(), "42");
        assert_eq!(
            io.input("again? "),
            Err(IoFailure::InputRequired("again? ".to_owned()))
        );
    }
}
