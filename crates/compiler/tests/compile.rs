use std::{error::Error, fs, io, path::PathBuf};

use oxyst_compiler::{CompileOutcome, Compiler, Error as CompilerError, Severity};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn read(name: &str) -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(fixtures().join(name))?)
}

#[test]
fn compiles_good_fixtures_and_reports_bad_source() -> Result<(), Box<dyn Error>> {
    let root = fixtures();
    let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;

    let CompileOutcome::Success(simple) = compiler.compile(&read("simple.typ")?) else {
        return Err("simple fixture failed to compile".into());
    };
    assert_eq!(simple.page_count(), 1);

    let CompileOutcome::Success(math) = compiler.compile(&read("math.typ")?) else {
        return Err("math fixture failed to compile".into());
    };
    assert_eq!(math.page_count(), 1);

    let CompileOutcome::Success(imported) = compiler.compile(&read("import.typ")?) else {
        return Err("import fixture failed to compile".into());
    };
    assert_eq!(imported.page_count(), 1);

    let CompileOutcome::Success(multi_page) = compiler.compile(&read("multi-page.typ")?) else {
        return Err("multi-page fixture failed to compile".into());
    };
    assert_eq!(multi_page.page_count(), 2);

    let CompileOutcome::Failure(diagnostics) = compiler.compile(&read("error.typ")?) else {
        return Err("invalid fixture unexpectedly compiled".into());
    };
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == Severity::Error && diagnostic.message.contains("expression")
    }));
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.line == Some(0))
    );

    Ok(())
}

#[test]
fn rejects_a_main_file_outside_the_project_root() {
    let root = fixtures();
    let outside = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let result = Compiler::new(&root, outside);

    assert!(matches!(result, Err(CompilerError::MainOutsideRoot { .. })));
}

#[test]
fn maps_between_source_and_preview_positions() -> Result<(), Box<dyn Error>> {
    let root = fixtures();
    let source = read("simple.typ")?;
    let cursor = source
        .find("Hello from")
        .ok_or_else(|| io::Error::other("fixture text is missing"))?;
    let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
    let CompileOutcome::Success(document) = compiler.compile(&source) else {
        return Err("simple fixture failed to compile".into());
    };

    let sync = document.sync();
    let position = sync
        .position_from_cursor(cursor)
        .ok_or_else(|| io::Error::other("cursor did not map to the preview"))?;
    assert_eq!(position.page, 0);
    assert!((0.0..=1.0).contains(&position.x));
    assert!((0.0..=1.0).contains(&position.y));
    assert!(sync.source_from_click(position).is_some());

    Ok(())
}

#[test]
fn incrementally_edits_the_source_used_by_snapshots() -> Result<(), Box<dyn Error>> {
    let root = fixtures();
    let source = read("simple.typ")?;
    let start = source
        .find("Hello")
        .ok_or_else(|| io::Error::other("fixture text is missing"))?;
    let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
    compiler.replace_source(&source);
    compiler.apply_edit(start..start + "Hello".len(), "Updated")?;

    let CompileOutcome::Success(document) = compiler.snapshot().compile() else {
        return Err("incrementally edited source failed to compile".into());
    };
    assert_eq!(document.page_count(), 1);
    Ok(())
}

#[test]
fn snapshots_keep_the_source_revision_they_were_created_from() -> Result<(), Box<dyn Error>> {
    let root = fixtures();
    let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
    compiler.replace_source("= Valid snapshot");
    let valid = compiler.snapshot();

    compiler.replace_source("#let broken =");
    let invalid = compiler.snapshot();

    assert!(matches!(valid.compile(), CompileOutcome::Success(_)));
    assert!(matches!(invalid.compile(), CompileOutcome::Failure(_)));
    Ok(())
}
