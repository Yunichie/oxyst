# Local patch

This directory contains the published source of `ratatui-image` 11.0.6:

https://github.com/ratatui/ratatui-image/tree/v11.0.6

The source is MIT-licensed; see `LICENSE`.

`src/sliced.rs` is patched so simultaneous top and bottom clipping:

- renders only the visible iTerm2 row slices;
- subtracts skipped Sixel bands from the visible band range.

Remove this vendored override after those fixes are included in a released
`ratatui-image` version and the preview scrolling regressions pass against it.
