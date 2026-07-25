#set page(width: 420pt, height: 595pt, margin: 36pt)
#set text(size: 10pt)

= Oxyst benchmark

This is a small Typst document with *emphasis*, a reference to @result, and
inline math $sum_(k=1)^n k = (n(n+1))/2$.

#figure(
  table(
    columns: 3,
    [Operation], [Input], [Result],
    [Compile], [Typst], [Document],
    [Render], [Document], [Pixels],
  ),
  caption: [A representative table.],
) <result>
