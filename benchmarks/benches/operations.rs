#![allow(clippy::panic, reason = "invalid benchmark setup must fail the run")]

use std::{fs, hint::black_box, path::PathBuf, time::Duration};

use criterion::{BatchSize, Criterion, SamplingMode, Throughput, criterion_group, criterion_main};
use oxyst_compiler::{CompileOutcome, CompiledDocument, Compiler, PagePosition};
use oxyst_config::Config;
use oxyst_document::{Document, Motion};
use oxyst_render::{
    ExportFormat, RenderCache, export, render, render_cached_cancellable, render_manifest,
    render_pages_cancellable,
};

const SMALL_SOURCE: &str = include_str!("../fixtures/small.typ");
const REALISTIC_SOURCE: &str = include_str!("../fixtures/realistic.typ");
const IMPORT_SOURCE: &str = include_str!("../fixtures/import.typ");
const INVALID_SOURCE: &str = "#let unfinished = (";
const EDIT_BATCH: usize = 1_000;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn compiler_for(main: &str) -> Compiler {
    let root = fixture_root();
    Compiler::new(&root, root.join(main))
        .unwrap_or_else(|error| panic!("could not create benchmark compiler: {error}"))
}

fn require_success(outcome: CompileOutcome) -> CompiledDocument {
    match outcome {
        CompileOutcome::Success(document) => document,
        CompileOutcome::Failure(diagnostics) => {
            panic!(
                "benchmark fixture failed to compile with {} diagnostics",
                diagnostics.len()
            )
        }
    }
}

fn require_failure(outcome: CompileOutcome) -> usize {
    match outcome {
        CompileOutcome::Failure(diagnostics) => diagnostics.len(),
        CompileOutcome::Success(_) => panic!("invalid benchmark fixture compiled successfully"),
    }
}

fn compile_realistic() -> CompiledDocument {
    require_success(compiler_for("realistic.typ").compile(REALISTIC_SOURCE))
}

fn editor_text() -> String {
    const LINE: &str = "The quick brown fox edits a representative Typst paragraph with words and numbers 12345.\n";
    LINE.repeat((256_usize * 1_024).div_ceil(LINE.len()))
}

fn unicode_line() -> String {
    "Latin 🙂 e\u{301} 中 हिन्दी العربية ".repeat(256)
}

fn paste_text() -> String {
    const FRAGMENT: &str = "A pasted Typst paragraph with words and x^2.\n";
    let mut text = FRAGMENT.repeat((4_usize * 1_024).div_ceil(FRAGMENT.len()));
    text.truncate(4 * 1_024);
    text
}

fn bench_config(c: &mut Criterion) {
    let path = fixture_root().join("config.toml");
    let mut group = c.benchmark_group("config");

    group.bench_function("defaults", |b| b.iter(|| black_box(Config::default())));
    group.bench_function("load_explicit_file", |b| {
        b.iter(|| {
            black_box(
                Config::load(Some(black_box(path.as_path())))
                    .unwrap_or_else(|error| panic!("could not load benchmark config: {error}")),
            )
        });
    });
    group.finish();
}

fn bench_document(c: &mut Criterion) {
    let text = editor_text();
    let text_bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
    let mut group = c.benchmark_group("document");

    group.throughput(Throughput::Bytes(text_bytes));
    group.bench_function("create_256_kib", |b| {
        b.iter(|| black_box(Document::new(black_box(&text))));
    });

    let document = Document::new(&text);
    group.bench_function("serialize_256_kib", |b| {
        b.iter(|| black_box(document.text()));
    });

    group.throughput(Throughput::Elements(1));
    group.bench_function("insert_character_middle", |b| {
        b.iter_batched(
            || {
                let mut document = Document::new(&text);
                assert!(document.set_cursor_byte_index(text.len() / 2));
                document
            },
            |mut document| {
                document.insert_char(black_box('x'));
                black_box(document);
            },
            BatchSize::LargeInput,
        );
    });

    let paste = paste_text();
    group.throughput(Throughput::Bytes(
        u64::try_from(paste.len()).unwrap_or(u64::MAX),
    ));
    group.bench_function("paste_4_kib_middle", |b| {
        b.iter_batched(
            || {
                let mut document = Document::new(&text);
                assert!(document.set_cursor_byte_index(text.len() / 2));
                document
            },
            |mut document| {
                document.insert_text(black_box(&paste));
                black_box(document);
            },
            BatchSize::LargeInput,
        );
    });

    group.throughput(Throughput::Bytes(64 * 1_024));
    group.bench_function("delete_selection_64_kib", |b| {
        b.iter_batched(
            || {
                let mut document = Document::new(&text);
                assert!(document.select_byte_range(64 * 1_024..128 * 1_024));
                document
            },
            |mut document| {
                assert!(document.delete_selection());
                black_box(document);
            },
            BatchSize::LargeInput,
        );
    });

    group.throughput(Throughput::Bytes(
        u64::try_from(paste.len()).unwrap_or(u64::MAX),
    ));
    group.bench_function("undo_paste_4_kib", |b| {
        b.iter_batched(
            || {
                let mut document = Document::new(&text);
                assert!(document.set_cursor_byte_index(text.len() / 2));
                document.insert_text(&paste);
                document
            },
            |mut document| {
                document.undo();
                black_box(document);
            },
            BatchSize::LargeInput,
        );
    });

    group.finish();

    let unicode = unicode_line();
    let mut cursor_group = c.benchmark_group("cursor");
    cursor_group.throughput(Throughput::Elements(EDIT_BATCH as u64));
    cursor_group.bench_function("move_by_grapheme_1000", |b| {
        b.iter_batched(
            || Document::new(&unicode),
            |mut document| {
                for _ in 0..EDIT_BATCH {
                    document.move_cursor(black_box(Motion::Right));
                }
                black_box(document.cursor_position());
            },
            BatchSize::SmallInput,
        );
    });
    cursor_group.bench_function("move_by_word_1000", |b| {
        b.iter_batched(
            || Document::new(&text),
            |mut document| {
                for _ in 0..EDIT_BATCH {
                    document.move_cursor(black_box(Motion::WordRight));
                }
                black_box(document.cursor_position());
            },
            BatchSize::SmallInput,
        );
    });
    cursor_group.finish();
}

fn bench_compiler_startup(c: &mut Criterion) {
    let root = fixture_root();
    let main = root.join("small.typ");
    let mut group = c.benchmark_group("compiler_startup");
    group
        .sample_size(30)
        .measurement_time(Duration::from_secs(10))
        .sampling_mode(SamplingMode::Flat);

    group.bench_function("initialize_and_compile_small", |b| {
        b.iter(|| {
            let mut compiler = Compiler::new(black_box(&root), black_box(&main))
                .unwrap_or_else(|error| panic!("could not create benchmark compiler: {error}"));
            let document = require_success(compiler.compile(black_box(SMALL_SOURCE)));
            black_box(document.page_count());
        });
    });
    group.finish();
}

fn bench_compiler(c: &mut Criterion) {
    let mut group = c.benchmark_group("compiler");

    let mut steady = compiler_for("realistic.typ");
    black_box(require_success(steady.compile(REALISTIC_SOURCE)));
    group.bench_function("recompile_unchanged_realistic", |b| {
        b.iter(|| black_box(steady.compile(black_box(REALISTIC_SOURCE))));
    });

    let mut incremental = compiler_for("realistic.typ");
    incremental.replace_source(REALISTIC_SOURCE);
    black_box(require_success(incremental.snapshot().compile()));
    let revision = REALISTIC_SOURCE
        .find("#let revision = 0")
        .map(|start| start + "#let revision = ".len())
        .unwrap_or_else(|| panic!("realistic fixture has no revision marker"));
    let mut next_revision = '1';
    group.bench_function("apply_single_edit_and_compile_realistic", |b| {
        b.iter(|| {
            let replacement = if next_revision == '1' { "1" } else { "0" };
            next_revision = if next_revision == '1' { '0' } else { '1' };
            incremental
                .apply_edit(revision..revision + 1, replacement)
                .unwrap_or_else(|error| panic!("could not apply benchmark edit: {error}"));
            black_box(incremental.snapshot().compile());
        });
    });

    let mut imported = compiler_for("import.typ");
    group.bench_function("invalidate_and_compile_import", |b| {
        b.iter(|| {
            imported.invalidate_files();
            black_box(imported.compile(black_box(IMPORT_SOURCE)));
        });
    });

    let mut invalid = compiler_for("small.typ");
    invalid.replace_source(INVALID_SOURCE);
    black_box(require_failure(invalid.snapshot().compile()));
    let invalid_edit = INVALID_SOURCE.len() - 1;
    let mut next_delimiter = ']';
    group.bench_function("apply_invalid_edit_and_compile_diagnostics", |b| {
        b.iter(|| {
            let replacement = if next_delimiter == ']' { "]" } else { "(" };
            next_delimiter = if next_delimiter == ']' { '(' } else { ']' };
            invalid
                .apply_edit(invalid_edit..invalid_edit + 1, replacement)
                .unwrap_or_else(|error| panic!("could not apply invalid benchmark edit: {error}"));
            black_box(require_failure(invalid.snapshot().compile()));
        });
    });

    let mut snapshot_compiler = compiler_for("realistic.typ");
    snapshot_compiler.replace_source(REALISTIC_SOURCE);
    group.bench_function("create_snapshot", |b| {
        b.iter(|| black_box(snapshot_compiler.snapshot()));
    });

    let large_text = editor_text();
    snapshot_compiler.replace_source(&large_text);
    let snapshot = snapshot_compiler.snapshot();
    group.throughput(Throughput::Bytes(
        u64::try_from(large_text.len()).unwrap_or(u64::MAX),
    ));
    group.bench_function("word_count_256_kib", |b| {
        b.iter(|| black_box(snapshot.word_count()));
    });

    group.finish();
}

fn sync_positions(
    sync: &oxyst_compiler::DocumentSync,
    source: &str,
) -> (Vec<usize>, Vec<PagePosition>) {
    let cursors: Vec<_> = source
        .char_indices()
        .filter_map(|(offset, character)| character.is_alphanumeric().then_some(offset))
        .step_by(8)
        .take(64)
        .collect();
    let pages = cursors
        .iter()
        .filter_map(|cursor| sync.position_from_cursor(*cursor))
        .collect();
    (cursors, pages)
}

fn bench_synchronization(c: &mut Criterion) {
    let document = compile_realistic();
    let sync = document.sync();
    let (cursors, pages) = sync_positions(&sync, REALISTIC_SOURCE);
    assert!(!pages.is_empty(), "fixture produced no preview positions");

    let mut group = c.benchmark_group("synchronization");
    group.throughput(Throughput::Elements(
        u64::try_from(cursors.len()).unwrap_or(u64::MAX),
    ));
    group.bench_function("cursor_to_preview_64", |b| {
        b.iter(|| {
            let found = cursors
                .iter()
                .filter_map(|cursor| sync.position_from_cursor(black_box(*cursor)))
                .count();
            black_box(found);
        });
    });

    group.throughput(Throughput::Elements(
        u64::try_from(pages.len()).unwrap_or(u64::MAX),
    ));
    group.bench_function("preview_to_source", |b| {
        b.iter(|| {
            let found = pages
                .iter()
                .filter_map(|position| sync.source_from_click(black_box(*position)))
                .count();
            black_box(found);
        });
    });
    group.finish();
}

fn bench_render(c: &mut Criterion) {
    let document = compile_realistic();
    let manifest = render_manifest(&document, 120)
        .unwrap_or_else(|error| panic!("could not create benchmark manifest: {error}"));
    let (_, warm_cache) =
        render_cached_cancellable(&document, 120, &RenderCache::default(), || false)
            .unwrap_or_else(|error| panic!("could not warm benchmark render cache: {error}"));
    let mut group = c.benchmark_group("render");

    group.throughput(Throughput::Elements(
        u64::try_from(document.page_count()).unwrap_or(u64::MAX),
    ));
    group.bench_function("manifest_120_px", |b| {
        b.iter(|| {
            black_box(
                render_manifest(black_box(&document), black_box(120))
                    .unwrap_or_else(|error| panic!("could not render manifest: {error}")),
            )
        });
    });
    group.bench_function("all_pages_120_px_uncached", |b| {
        b.iter(|| {
            black_box(
                render(black_box(&document), black_box(120))
                    .unwrap_or_else(|error| panic!("could not render document: {error}")),
            )
        });
    });
    group.throughput(Throughput::Elements(1));
    group.bench_function("first_page_120_px_uncached", |b| {
        b.iter(|| {
            black_box(
                render_pages_cancellable(black_box(&document), black_box(&manifest), [0], || false)
                    .unwrap_or_else(|error| panic!("could not render page: {error}")),
            )
        });
    });
    group.throughput(Throughput::Elements(
        u64::try_from(document.page_count()).unwrap_or(u64::MAX),
    ));
    group.bench_function("all_pages_120_px_cached", |b| {
        b.iter(|| {
            black_box(
                render_cached_cancellable(
                    black_box(&document),
                    black_box(120),
                    black_box(&warm_cache),
                    || false,
                )
                .unwrap_or_else(|error| panic!("could not render cached document: {error}")),
            )
        });
    });
    group.finish();
}

fn bench_export(c: &mut Criterion) {
    let document = compile_realistic();
    let output_root =
        std::env::temp_dir().join(format!("oxyst-benchmarks-export-{}", std::process::id()));
    fs::create_dir_all(&output_root)
        .unwrap_or_else(|error| panic!("could not create export directory: {error}"));

    let mut group = c.benchmark_group("export");
    for format in [ExportFormat::Pdf, ExportFormat::Png, ExportFormat::Svg] {
        let path = output_root.join(format!("document.{}", format.extension()));
        group.bench_function(format.label(), |b| {
            b.iter(|| {
                export(black_box(&document), black_box(format), black_box(&path))
                    .unwrap_or_else(|error| panic!("could not export benchmark document: {error}"));
                black_box(
                    fs::metadata(&path)
                        .unwrap_or_else(|error| panic!("could not inspect export: {error}"))
                        .len(),
                );
            });
        });
    }
    group.finish();

    for format in [ExportFormat::Pdf, ExportFormat::Png, ExportFormat::Svg] {
        let path = output_root.join(format!("document.{}", format.extension()));
        let _ = fs::remove_file(path);
    }
    let _ = fs::remove_dir(output_root);
}

criterion_group!(
    operations,
    bench_config,
    bench_document,
    bench_compiler_startup,
    bench_compiler,
    bench_synchronization,
    bench_render,
    bench_export
);
criterion_main!(operations);
