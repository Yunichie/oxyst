use std::{error::Error, fs, io, path::PathBuf};

use typst_tui_compiler::{CompileOutcome, Compiler};
use typst_tui_render::{ExportFormat, export};

#[test]
fn renders_compiled_page_pixels() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let main = root.join("simple.typ");
    let source = fs::read_to_string(&main)?;
    let mut compiler = Compiler::new(&root, &main)?;
    let CompileOutcome::Success(compiled) = compiler.compile(&source) else {
        return Err("simple fixture failed to compile".into());
    };

    let rendered = typst_tui_render::render(&compiled, 600)?;
    assert_eq!(rendered.pages().len(), 1);
    let page = &rendered.pages()[0];
    assert_eq!(page.width(), 600);
    assert!(page.height() > 0);
    assert!(
        page.rgba()
            .chunks_exact(4)
            .any(|pixel| pixel[..3] != [255, 255, 255])
    );

    Ok(())
}

#[test]
fn exports_all_supported_formats() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let main = root.join("multi-page.typ");
    let source = fs::read_to_string(&main)?;
    let mut compiler = Compiler::new(&root, &main)?;
    let CompileOutcome::Success(document) = compiler.compile(&source) else {
        return Err(io::Error::other("multi-page fixture did not compile").into());
    };
    let output_root =
        std::env::temp_dir().join(format!("typst-tui-export-test-{}", std::process::id()));
    fs::create_dir_all(&output_root)?;

    for format in [ExportFormat::Pdf, ExportFormat::Png, ExportFormat::Svg] {
        let path = output_root.join(format!("document.{}", format.extension()));
        export(&document, format, &path)?;
        assert!(fs::metadata(path)?.len() > 0);
    }

    fs::remove_dir_all(output_root)?;
    Ok(())
}
