use typst::{World, WorldExt, diag::SourceDiagnostic};

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
    let id = diagnostic.span.id();
    let position = id
        .and_then(|id| world.source(id).ok())
        .zip(world.range(diagnostic.span))
        .and_then(|(source, range)| source.lines().byte_to_line_column(range.start));
    let path = id.map(|id| match id.root() {
        typst::syntax::VirtualRoot::Project => id.vpath().get_without_slash().to_owned(),
        typst::syntax::VirtualRoot::Package(package) => {
            format!("{package}{}", id.vpath().get_with_slash())
        }
    });

    Diagnostic {
        severity: match diagnostic.severity {
            typst::diag::Severity::Error => Severity::Error,
            typst::diag::Severity::Warning => Severity::Warning,
        },
        message: diagnostic.message.into(),
        path,
        line: position.map(|(line, _)| line),
        column: position.map(|(_, column)| column),
    }
}
