use std::{error::Error, fs, path::PathBuf};

use typst_tui_compiler::{CompileOutcome, Compiler, Error as CompilerError, Severity};

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
