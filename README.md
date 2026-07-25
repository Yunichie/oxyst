# oxyst

`oxyst` is a terminal editor for Typst documents with syntax highlighting, background compilation,
and a live in-terminal preview.

## Requirements

- A terminal supported by Crossterm
- Rust 1.92 or newer and Cargo to build from source

The Typst compiler is built in; the `typst` executable is not required.

## Installation

Install the latest version directly from GitHub:

```console
cargo install --git https://github.com/Yunichie/oxyst.git
```

To install from a local checkout:

```bash
git clone https://github.com/Yunichie/oxyst.git
cd oxyst
cargo install --path .
```

Upgrade an existing installation by adding `--force` to either `cargo install` command. Verify the
installation with:

```bash
oxyst --version
```

Cargo installs the binary in `$CARGO_HOME/bin`, which defaults to `$HOME/.cargo/bin` on Unix and
`%USERPROFILE%\.cargo\bin` on Windows. Add that directory to `PATH` if `oxyst` is not found.

## Configuration

`oxyst` loads the first applicable configuration file:

1. The path passed with `--config FILE`
2. The path in `OXYST_CONFIG`
3. `$XDG_CONFIG_HOME/oxyst/config.toml`
4. `%APPDATA%\oxyst\config.toml` on Windows
5. `$HOME/.config/oxyst/config.toml`

The file is optional and uses TOML. Values are merged over the defaults, so only overrides are
needed:

```toml
theme = "light"

[keys]
save = ["alt+s"]
command_palette = ["ctrl+shift+p", ":"]
toggle_file_explorer = ["ctrl+e"]
quit = []
```

Supported themes (for now) are `dark` and `light`. Override the configured theme for one run with
`--theme dark` or `--theme light`.

## License

Licensed under either the [MIT License](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option.
