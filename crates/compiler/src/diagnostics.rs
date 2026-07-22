use typst::{World, WorldExt, diag::SourceDiagnostic, syntax::DiagSpan};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub path: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub is_main: bool,
    pub notes: Vec<DiagnosticNote>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticNoteKind {
    Hint,
    Trace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticNote {
    pub kind: DiagnosticNoteKind,
    pub message: String,
    pub path: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub is_main: bool,
}

pub(crate) fn convert_diagnostics(
    world: &dyn World,
    diagnostics: impl IntoIterator<Item = SourceDiagnostic>,
) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| convert_diagnostic(world, diagnostic))
        .collect()
}

fn convert_diagnostic(world: &dyn World, diagnostic: SourceDiagnostic) -> Diagnostic {
    let location = locate(world, diagnostic.span);
    let notes = diagnostic
        .trace
        .into_iter()
        .map(|trace| {
            note(
                world,
                DiagnosticNoteKind::Trace,
                trace.v.to_string(),
                trace.span.into(),
            )
        })
        .chain(
            diagnostic
                .hints
                .into_iter()
                .map(|hint| note(world, DiagnosticNoteKind::Hint, hint.v.into(), hint.span)),
        )
        .collect();

    Diagnostic {
        severity: match diagnostic.severity {
            typst::diag::Severity::Error => Severity::Error,
            typst::diag::Severity::Warning => Severity::Warning,
        },
        message: diagnostic.message.into(),
        path: location.path,
        line: location.line,
        column: location.column,
        is_main: location.is_main,
        notes,
    }
}

struct Location {
    path: Option<String>,
    line: Option<usize>,
    column: Option<usize>,
    is_main: bool,
}

fn locate(world: &dyn World, span: DiagSpan) -> Location {
    let id = span.id();
    let position = id
        .and_then(|id| world.source(id).ok())
        .zip(world.range(span))
        .and_then(|(source, range)| source.lines().byte_to_line_column(range.start));
    let path = id.map(|id| match id.root() {
        typst::syntax::VirtualRoot::Project => id.vpath().get_without_slash().to_owned(),
        typst::syntax::VirtualRoot::Package(package) => {
            format!("{package}{}", id.vpath().get_with_slash())
        }
    });

    Location {
        path,
        line: position.map(|(line, _)| line),
        column: position.map(|(_, column)| column),
        is_main: id == Some(world.main()),
    }
}

fn note(
    world: &dyn World,
    kind: DiagnosticNoteKind,
    message: String,
    span: DiagSpan,
) -> DiagnosticNote {
    let location = locate(world, span);
    DiagnosticNote {
        kind,
        message,
        path: location.path,
        line: location.line,
        column: location.column,
        is_main: location.is_main,
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf};

    use typst::{
        diag::{SourceDiagnostic, Tracepoint},
        syntax::Spanned,
    };

    use super::{DiagnosticNoteKind, convert_diagnostics};
    use crate::world::TypstWorld;

    #[test]
    fn preserves_typst_hints_and_tracepoints() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let mut world = TypstWorld::new(&root, &root.join("simple.typ"))?;
        world.replace_source("#let value = 1");
        let span = world.main_source().root().span();
        let mut diagnostic = SourceDiagnostic::error(span, "broken value").with_hint("use text");
        diagnostic
            .trace
            .push(Spanned::new(Tracepoint::Call(Some("helper".into())), span));

        let converted = convert_diagnostics(&world, [diagnostic]);
        assert_eq!(converted.len(), 1);
        assert!(
            converted[0].notes.iter().any(|note| {
                note.kind == DiagnosticNoteKind::Hint && note.message == "use text"
            })
        );
        assert!(converted[0].notes.iter().any(|note| {
            note.kind == DiagnosticNoteKind::Trace && note.message.contains("helper")
        }));
        Ok(())
    }
}
