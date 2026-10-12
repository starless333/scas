use scas::notebook::{CellState, NotebookSession, OutputKind};

const FIXTURE: &str = include_str!("../examples/notebook-phase1.scas");

#[test]
fn fixture_emits_ordered_typed_events_and_manifest() {
    let mut session = NotebookSession::new(FIXTURE);
    let run = session.run_all();
    assert!(run.clean);
    assert!(run.states.iter().all(|state| *state == CellState::Current));
    assert!(run
        .events
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence));
    assert!(run
        .events
        .iter()
        .any(|event| event.kind == OutputKind::Text));
    assert!(run
        .events
        .iter()
        .any(|event| event.kind == OutputKind::Math));
    assert!(run
        .events
        .iter()
        .any(|event| event.kind == OutputKind::Stdout && event.text == "36\n"));
    assert!(run
        .events
        .iter()
        .any(|event| event.kind == OutputKind::Verification));
    let manifest = run.manifest.expect("clean run has a manifest");
    assert_eq!(manifest.source_sha256.len(), 64);
    assert_eq!(manifest.verification.passed, 1);
}

#[test]
fn run_above_resets_state_and_cell_runs_are_interactive() {
    let mut session = NotebookSession::new(FIXTURE);
    let last = session.cells().len() - 1;
    let run = session.run_above(last);
    assert!(run.succeeded());
    assert!(!run.clean);
    assert!(run.manifest.is_none());

    let first = session.run_cell(0);
    assert!(first.succeeded());
    assert!(!first.clean);
    assert!(session
        .states()
        .iter()
        .skip(1)
        .all(|state| *state == CellState::Stale));
}
