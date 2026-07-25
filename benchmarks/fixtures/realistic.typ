#set page(width: 420pt, height: 595pt, margin: 36pt)
#set text(size: 10pt)
#set par(justify: true)
#let revision = 0

= Oxyst performance report

This benchmark document combines the content commonly edited in Typst:
headings, paragraphs, lists, tables, references, code, and mathematics. The
revision marker is #revision.

== Background

Oxyst compiles documents while the user edits and renders the result in a
terminal preview. #lorem(140)

- Responsive document editing
- Incremental compilation
- Source and preview synchronization
- Cached page rendering

#figure(
  table(
    columns: (1fr, 1fr, 1fr),
    align: (left, center, right),
    [Stage], [Input], [Output],
    [Edit], [Text], [Revision],
    [Compile], [Source], [Pages],
    [Render], [Pages], [Pixels],
  ),
  caption: [The main processing stages.],
) <pipeline>

#pagebreak()

= Compilation

The compiler owns the project world, font collection, package access, and
source snapshots. #lorem(180)

For a sequence $a_1, dots, a_n$, the arithmetic mean is
$overline(a) = 1/n sum_(k=1)^n a_k$.

```rust
let snapshot = compiler.snapshot();
let outcome = snapshot.compile();
```

See @pipeline for the complete flow. #lorem(100)

#pagebreak()

= Rendering

Preview rendering computes page metadata before rasterizing requested pages.
The cache reuses pages whose fingerprints and scale are unchanged.
#lorem(220)

#figure(
  rect(
    width: 100%,
    inset: 12pt,
    fill: rgb("#eef6ff"),
    stroke: rgb("#4285c5"),
    [A representative rendered panel with enough structure to exercise fills,
     strokes, text shaping, and layout.],
  ),
  caption: [A styled preview element.],
)

#pagebreak()

= Diagnostics and export

Diagnostics map compiler spans back to source lines. Export writes the same
compiled document as PDF, PNG, or SVG. #lorem(220)

1. Validate the source.
2. Compile a paged document.
3. Synchronize source positions.
4. Render or export the result.

#align(center)[
  *End of benchmark document*
]
