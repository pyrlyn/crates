# d2-render

Render [D2](https://d2lang.com) diagrams from Rust.

By default the crate renders in-process with a pure-Rust port of a subset of
D2: no `d2` binary, no subprocess, no Go or JavaScript. Enable the `cli`
feature to render through the official `d2` executable instead, with every
D2 feature, layout engine (dagre, ELK, TALA) and output format. Either way
you get:

- a typed API: `Renderer`, `Format`, `RenderOptions`, `RenderReport`;
- structured errors: `Diagnostic`s with file, line and column;
- SVG metadata: size, `viewBox`, and the key of every shape and connection
  in the rendered SVG;
- freshness checks: stamp outputs with a source hash, then find and
  re-render the outdated ones;
- optional features: PNG from the native backend (`png`, pure-Rust
  rasterizer), the `d2` CLI backend (`cli`), a ratatui widget (`tui`), HTML
  embedding helpers (`web`) and debounced re-render on change (`watch`).

## Usage

```toml
[dependencies]
d2-render = "0.1"                                     # native backend, SVG
# d2-render = { version = "0.1", features = ["png"] } # + PNG, still pure Rust
# d2-render = { version = "0.1", features = ["cli"] } # + the d2 executable backend
```

```rust
use std::path::Path;
use d2_render::{Error, Format, RenderOptions, Renderer};

fn main() -> Result<(), Error> {
    // NativeBackend: nothing to install.
    let renderer = Renderer::new().options(RenderOptions::new().theme(200).pad(20));

    // From a file. The format comes from the enum, not from the extension.
    let svg = renderer.render_file(Path::new("arch.d2"), Path::new("out/arch.svg"), Format::Svg)?;

    // From a string.
    renderer.render_str("a -> b: hello", Path::new("out/ab.svg"), Format::Svg)?;

    // With a full report.
    let report = renderer.render_file_report(Path::new("arch.d2"), &svg, Format::Svg)?;
    println!("{report}"); // rendered arch.d2 -> out/arch.svg (svg, native backend) in 1.2ms; 315x728, ...
    for w in &report.warnings {
        eprintln!("ignored: {w}"); // D2 features the native backend does not draw
    }

    // Compile errors carry positions.
    match renderer.render_str("a: {", Path::new("out/bad.svg"), Format::Svg) {
        Err(Error::Syntax { diagnostics, .. }) => {
            for d in diagnostics {
                eprintln!("{}:{}: {}", d.line.unwrap_or(0), d.column.unwrap_or(0), d.message);
            }
        }
        other => println!("{other:?}"),
    }
    Ok(())
}
```

With the `cli` feature the same calls go through the `d2` executable:

```rust
let renderer = Renderer::cli()                        // $D2_BIN, else `d2` on PATH
    .options(RenderOptions::new().layout(Layout::Elk));
let renderer = Renderer::with_binary("/opt/d2/bin/d2"); // explicit binary
```

`Format` covers everything d2 writes: `Svg`, `Png`, `Pdf`, `Pptx`, `Gif`,
`Txt`. The native backend writes `Svg`, and `Png` with the `png` feature;
other formats return `Error::UnsupportedFormat`. When the output path has
another extension, the render goes to a temporary file beside it and is
then moved into place, so the file always holds the requested format.
`Renderer::render_auto` infers the format from the output extension
instead. Missing parent directories are created.

## Why native is the default, and when to enable `cli`

Native is the default because it makes the crate self-contained. `cargo add
d2-render` works on any machine, in CI, in sandboxes and in containers. No
binary has to be installed, found on `PATH` or kept at the right version.
Nothing is spawned, so a render takes about a millisecond and cannot hang
or leak a process. The same source always gives the same output for a given
crate version. All of that comes at the cost of covering a subset of D2
(see the table below). Everything the native backend skips is reported, not
silently dropped: either a warning in `RenderReport::warnings` or, for
constructs whose meaning it cannot preserve, an `Error::Syntax`
diagnostic.

Enable `cli` (and call `Renderer::cli()`) when you need any of:

- output that has to match `d2` exactly (same layout, same fonts);
- ELK or TALA layouts, sketch mode, dark themes, animations, `--target`;
- PDF, PPTX, GIF or ASCII (`Txt`) output;
- sequence diagrams, `sql_table`, `class` shapes, imports, globs, layers,
  scenarios or steps, Markdown/LaTeX rendering, remote icons bundled into
  the SVG;
- `d2 fmt` (`CliBackend::format_file`, `CliBackend::is_formatted`).

A common split is native in tests and previews, CLI in the build step that
publishes diagrams. Both implement `Backend`, so `Renderer::with_backend`
can pick one at run time, and the freshness, report, `tui`, `web` and
`watch` APIs work the same with either.

## What the native backend supports

| Area | Supported | Not supported (what happens) |
| --- | --- | --- |
| Shapes | rectangle, square, circle, oval, diamond, hexagon, parallelogram, cylinder, queue, page, document, step, package, stored_data, person, c4-person, cloud, callout, text, code (monospace), hierarchy (as rectangle), image | `class`, `sql_table`, `sequence_diagram`: drawn as rectangles, with a warning |
| Labels | plain and quoted labels, quoted keys, block strings `\|md ...\|` shown as plain text, `style.text-transform` | Markdown/LaTeX rendering (shown as plain text); `label.near` / `icon.near` positioning (warning) |
| Structure | nested containers, `_` parent references, `direction`, `width`/`height`, grids (`grid-rows`, `grid-columns`, `grid-gap`, `vertical-gap`, `horizontal-gap`) | imports `@file`, globs `*`, `(a -> b)[0]` references (diagnostic); layers, scenarios, steps (warning) |
| Connections | `->` `<-` `<->` `--`, chains, labels, repeated connections, arrowhead shapes (triangle, arrow, diamond, circle, box, cross, cf-one, cf-one-required, cf-many, cf-many-required), `style.filled`, arrowhead labels | `label.near` on connections (warning) |
| Variables | `vars` with nested maps, scoping, `${a.b}` substitution (not inside single quotes), the `could not resolve variable` diagnostic | spreads `...${x}` (diagnostic) |
| `d2-config` | `theme-id`, `pad` (render options take precedence) | `dark-theme-id`, `sketch`, `center`, `layout-engine` other than dagre, `theme-overrides`, ... (warning) |
| Classes | `classes`, `class: name`, `class: [a; b]` on shapes and connections; later classes win, explicit fields win over classes | an undefined class is a warning (d2 keeps it as a CSS class only); class names are written to the SVG `class` attribute as d2 does |
| `near` | the 8 constants on top-level shapes (`top-left` ... `bottom-right`), laid out around the diagram | `near` an object, constants on nested shapes (warning); a missing target is the same diagnostic as d2 |
| Icons, links | `icon` drawn as an `<image href>` reference (relative paths and URLs, not bundled), `shape: image`, `tooltip` (`<title>`), `link` (`<a href>`) | remote icons are not fetched or embedded; PNG output only shows local files |
| Style | `fill`, `stroke`, `stroke-width`, `stroke-dash`, `opacity`, `border-radius`, `font-size`, `font-color`, `font: mono`, `bold`, `italic`, `underline`, `text-transform`, `shadow`, `multiple`, `double-border`, `3d` (rectangle, square), `filled` | `animated` (drawn static), `fill-pattern` and other keywords (warning) |
| Themes | all 20 catalog themes (`--theme` / `theme-id`), with the Terminal and Origami `Mono`, `CapsLock` and outer double-border rules | `--dark-theme`, `ContainerDots`, `AllPaper` and C4 rules (warning) |
| Layout | one dagre-style layered layout; edges routed between containers | ELK, TALA, `--center`, `--sketch`, `top`/`left` positions; positions differ from d2's |
| Output | SVG; PNG with feature `png` (resvg, pure Rust, `scale` = pixel ratio, default 2) | PDF, PPTX, GIF, ASCII (`Error::UnsupportedFormat`); `--target`, `--animate-interval` |
| Diagnostics | d2's message, line and column for parse errors, unknown shapes, bad style values, missing connection ends, undefined vars/classes, bad `near` targets | errors d2 detects only in later stages |

Element keys match d2's, so `SvgInfo` lists the same shapes and
connections as for CLI output. With feature `cli` and d2 installed, the
test suite checks this, and the diagnostics, against the real CLI.

### Options

Every field of `RenderOptions` maps to a flag that `d2 --help` lists (v0.9).
The CLI backend passes nothing you did not set, so d2's own defaults and
environment variables (`D2_THEME`, `D2_LAYOUT`, ...) still apply. The
native backend honours `theme`, `pad`, `salt`, `no_xml_tag`,
`omit_version` and (for PNG) `scale`; the others are reported in
`RenderReport::warnings` and ignored.

| Builder | d2 flag |
| --- | --- |
| `theme(i64)` / `dark_theme(i64)` | `--theme` / `--dark-theme` |
| `layout(Layout::Dagre \| Elk \| Tala \| Other(..))` | `--layout` |
| `pad(u32)` | `--pad` |
| `sketch(bool)` | `--sketch` |
| `center(bool)` | `--center` |
| `scale(f64)` | `--scale` |
| `timeout_secs(u32)` | `--timeout` |
| `target(str)` | `--target` |
| `no_xml_tag(bool)` | `--no-xml-tag` |
| `omit_version(bool)` | `--omit-version` |
| `salt(str)` | `--salt` |
| `animate_interval_ms(u32)` | `--animate-interval` |
| `stamp(bool)` | none: records a source hash, see Freshness below |

The CLI backend always adds `--watch=false`, so a `D2_WATCH` in the
environment cannot turn a render into a server that never exits.

### Errors

```rust
pub enum Error {
    BinaryNotFound { binary: PathBuf, reason: String }, // cli: d2 not installed / not executable
    Syntax { diagnostics: Vec<Diagnostic>, stderr: String }, // compile errors, with positions
    Io(std::io::Error),
    Failed { code: Option<i32>, messages: Vec<String>, stderr: String, partial: bool }, // cli: any other non-zero exit
    UnsupportedFormat { format: Format, backend: &'static str },
    UnknownFormat(PathBuf),
    Watch(String),
    Raster(String), // png: rasterization failed
}
```

`Failed` covers bad flags, an unknown theme or layout, timeouts, and
failures such as a remote icon that cannot be fetched; `partial` is set when
d2 still wrote a partial render.

### Structured output

The native backend produces `Diagnostic`s directly, with the same message,
line and column d2 reports for the errors it checks. The d2 CLI has no
machine-readable output mode (no JSON flag in v0.9), so the CLI backend
parses its stderr. `parse_stderr` is public if you run d2 yourself:

- `err: failed to compile x.d2: /p/x.d2:5:4: maps must be terminated with }`
  becomes `Diagnostic { severity: Error, path: Some("/p/x.d2"), line: Some(5), column: Some(4), message: "maps must be terminated with }" }`;
  every located error is kept (d2 reports several semantic errors at once);
- `warn:` lines become warnings; `success: ... in 4.39ms` gives
  `reported_duration`; `partial render written` sets `partial`.

A successful render returns a `RenderReport`: input, output, format,
duration, backend, warnings, diagnostics, `source_hash`, raw stderr, and for
SVG an `SvgInfo`. `SvgInfo::parse` reads the root `viewBox` and size,
`data-d2-version`, every `id`, and the key of every shape and connection.
Both backends write those keys as base64 in `<g class="...">` (`YQ==` is `a`,
`KGEgLSZndDsgYilbMF0=` is `(a -> b)[0]`); they come back decoded and split
into `shapes()` and `connections()`. `RenderReport` and `SvgInfo` both
implement `Display`.

### Validation and formatting

- `Renderer::validate_file` / `validate_str` parse and compile without
  drawing. With the native backend this is in-process. With the CLI backend
  it runs `d2 validate`, then a full compile to stdout
  (`--stdout-format=svg ... -`, with `--bundle=false` so no network is
  touched). Both steps are needed: in v0.9 `d2 validate` only parses, so an
  unknown shape or `opacity: 7` passes it but fails a render.
- `cli`: `CliBackend::validate_syntax` is the `d2 validate` step on its own.
- `cli`: `CliBackend::format_file` runs `d2 fmt`; `CliBackend::is_formatted` runs
  `d2 fmt --check`.

### Freshness

With `RenderOptions::stamp(true)` each render records
`sha256(source + options fingerprint)`. SVG gets it in a trailing
`<!-- d2-render:source-sha256=... -->` comment; other formats get a
`<output>.d2hash` sidecar file. Then:

```rust
let r = Renderer::new().options(RenderOptions::new().stamp(true));
r.is_stale(Path::new("a.d2"), Path::new("a.svg"))?;  // bool
r.freshness(Path::new("a.d2"), Path::new("a.svg"))?; // Fresh | Missing | Unstamped | Outdated { .. }
let stale = r.verify_dir(Path::new("docs"), Format::Svg)?; // every docs/**/x.d2 whose x.svg is stale
let reports = r.refresh_dir(Path::new("docs"), Format::Svg)?; // re-render only those
```

Changing a render option (theme, layout, ...) also makes an output stale.
Only the main file is hashed, so changes in files it imports are not
detected.

## Cargo features

| Feature | Adds | Extra dependencies |
| --- | --- | --- |
| *(always)* | `Renderer`, `Backend`, diagnostics, reports, SVG metadata, freshness | `sha2`, `tempfile` |
| `native` (default) | `NativeBackend`, `Renderer::new`/`native`, `native::render_svg`, `native::check` | none |
| `png` | PNG output from the native backend (implies `native`) | `resvg` |
| `cli` | `CliBackend`, `Renderer::cli`/`with_binary`, `D2_BIN_ENV`; `Renderer::new` uses it when `native` is off | none (needs the `d2` binary at run time) |
| `tui` | `tui::RenderStatus`, `tui::RenderStatusWidget` (ratatui `Widget`) | `ratatui` (no default features) |
| `web` | `web::inline_svg`, `data_uri`, `img_tag`, `html_page`, `write_html_page` | none |
| `watch` | `watch::watch`, `watch::watch_channel` | `notify`, `notify-debouncer-mini` |
| `full` | all of the above | |

With `default-features = false` and neither `native` nor `cli`, only
`Renderer::with_backend` is available, for your own `Backend`.

### `native`

```rust
let out = d2_render::native::render_svg("a -> b: hi", &RenderOptions::default())?;
println!("{} bytes, ignored: {:?}", out.svg.len(), out.warnings);
d2_render::native::check("a -> b")?; // parse + compile only
```

This is an in-process Rust rewrite of part of D2's pipeline: parser,
compiler, a dagre-style layered layout and an SVG writer. No d2 binary, Go
or JS is involved, and nothing touches the network. Positions come from a
different layout algorithm than d2's, so diagrams look similar, not
identical.

### `png`

```rust
let r = Renderer::new().options(RenderOptions::new().scale(2.0)); // pixel ratio, default 2
r.render_str("a -> b", Path::new("out/ab.png"), Format::Png)?;
```

The SVG from the native backend is rasterized with
[resvg](https://crates.io/crates/resvg) using the system fonts, loaded once
per process. Relative icon paths resolve against the input file's
directory; remote icons are left out.

### `cli`

The binary comes from, in order: `Renderer::with_binary(path)`, then the
`D2_BIN` environment variable, then `d2` on `PATH`. A missing binary is
reported as `Error::BinaryNotFound` by whichever call needs it. v0.9 is what
the tests run against:

```bash
brew install d2                                   # macOS / Linux (Homebrew)
curl -fsSL https://d2lang.com/install.sh | sh -s --  # other platforms
```

The CLI backend always adds `--watch=false`, so a `D2_WATCH` in the
environment cannot turn a render into a server that never exits.

### `tui`

```rust
let status = RenderStatus::from_result(Some(input), &renderer.render_file_report(&input, &out, Format::Svg));
frame.render_widget(RenderStatusWidget::new(&status).title("diagram"), area);
```

The widget shows the render state (idle, rendering, done, failed), the
input and output paths, SVG size and element counts, warnings, and each
diagnostic as `line:col message`. It does not preview the image in the
terminal: that needs a rasteriser plus a graphics-protocol crate, which
would outweigh the rest of this crate.

### `web`

```rust
let html = web::inline_svg(&svg_text, None);       // <div class="d2-diagram"><svg ...></div>, XML prolog stripped
let uri = web::data_uri(&png_bytes, Format::Png);  // data:image/png;base64,...
let page = web::html_page("Architecture", &svg_text); // standalone HTML document
```

To put several SVGs on one page, give each one a different
`RenderOptions::salt` so their element ids do not clash.

### `watch`

```rust
let config = WatchConfig { format: Format::Svg, out_dir: None, debounce: Duration::from_millis(200), render_on_start: true };
let (handle, events) = watch::watch_channel(Renderer::new(), &[PathBuf::from("docs")], config)?;
for event in events {
    if let WatchEvent::Rendered { input, result, .. } = event { /* ... */ }
}
// or: watch::watch(renderer, &paths, config, |event| { ... })
```

Directories are watched recursively for `*.d2`. A single file is watched
through its parent directory, so editors that save by renaming still
trigger a render. Outputs go next to their sources, or under `out_dir`
with the same relative layout. Dropping the `WatchHandle` stops the
watcher.

## Examples

```bash
cargo run -p d2-render --example basic -- diagram.d2 out.svg
cargo run -p d2-render --example native -- diagram.d2 out.svg
cargo run -p d2-render --features png --example png -- diagram.d2 out.png
cargo run -p d2-render --features cli --example cli -- diagram.d2 out.pdf
cargo run -p d2-render --features tui --example tui_report
cargo run -p d2-render --features web --example web_page
cargo run -p d2-render --features watch --example watch -- docs/
```

## Library routes considered

Before the native port, the question was whether D2 itself could run as a
library instead of a subprocess. These options were checked against the D2
source (Go module `github.com/d2lang/d2`, formerly `oss.terrastruct.com/d2`)
with real builds (Go 1.27 toolchain, wasmtime 49, October 2026):

| | `d2` CLI (feature `cli`) | Go library through cgo `c-shared` + FFI | Official JS/WASM build (`d2js`, npm `@d2lang/d2`, formerly `@terrastruct/d2`) | Whole CLI as WASI (`GOOS=wasip1`) under wasmtime | Pure-Rust (`native`, the default here; or the `d2-little` crate) |
| --- | --- | --- | --- | --- | --- |
| Feature parity | Complete | Complete | Compile + render to SVG or ASCII; no PNG/PDF | Complete: SVG, PNG, ELK all rendered in the check | Subset (see above) |
| Layouts | dagre, ELK, TALA | dagre, ELK, TALA | dagre, ELK, TALA | dagre, ELK, TALA (dagre and ELK checked) | one dagre-style layered layout |
| Build needs | nothing | Go ≥ 1.27 at build time, hand-written `//export` Go glue (D2 has no C API), a C linker per target, `unsafe` FFI (this repo forbids `unsafe_code`) | `GOOS=js GOARCH=wasm` + `syscall/js`, so it needs Go's `wasm_exec.js` glue and a JS engine | Go ≥ 1.27 to build a 73.5 MB `d2.wasm` from the D2 sources | nothing |
| Runtime needs | `d2` on PATH | Go runtime in-process (the native d2 binary is 56 MB) | Node/Deno/browser; wasmtime/wasmer cannot run `GOOS=js` modules | wasmtime (or the wasmtime crate, a large dependency); first run about 8 s to compile the module, then about 0.5 s per call with the cache | nothing |
| Version drift | whatever d2 is installed; `version()` reports it | pinned at build time | pinned to the npm release | pinned to the `.wasm` you ship | the rewrite has to be kept in sync by hand |
| Ergonomics in Rust | subprocess + stderr parsing (done here) | FFI layer, C ABI, cross-compiling Go | embedding a JS engine | subprocess (same as the CLI) or embedding wasmtime | plain Rust |

Every route that embeds D2 itself needs either the Go toolchain plus the D2
sources at build time, or a JS engine, and still pins a D2 version. So the
crate offers the two ends: its own Rust port as the default (no build or
runtime dependency on the D2 repository), and the real CLI behind `cli`
for full parity. The `Backend` trait keeps the choice pluggable.

The WASI build does work. If you already have a `d2.wasm` built with
`GOOS=wasip1 GOARCH=wasm go build`, the CLI backend (feature `cli`) can run it through a
two-line wrapper, because it passes absolute paths and only needs a binary
that behaves like `d2`:

```sh
#!/bin/sh
exec wasmtime run --dir /::/ /path/to/d2.wasm "$@"
```

```rust
let renderer = Renderer::with_binary("/usr/local/bin/d2-wasm"); // or D2_BIN=/usr/local/bin/d2-wasm
```

## Testing

```bash
cargo test -p d2-render                  # default features: never needs d2
cargo test -p d2-render --all-features   # adds CLI tests and CLI parity checks
cargo clippy -p d2-render --all-targets --all-features -- -D warnings
```

Tests with default features never run d2. Tests that need a real d2 are
compiled only with `cli`, print `SKIP <name>: d2 not available` and pass
when it is missing. PNG tests of the CLI backend skip themselves if d2
reports a missing browser (older d2 releases rendered PNG through
Playwright). The CLI error mapping and the watcher are also tested against
scripted stand-ins, so they run everywhere.

## License

`license = "MIT AND MPL-2.0"`. Code written for this crate is under the MIT
license ([LICENSE-MIT](https://github.com/pyrlyn/crates/blob/main/rust/d2-render/LICENSE-MIT)).
The files under `src/native/` are a Rust rewrite of parts of D2 (Copyright
2022 Terrastruct, Inc.) and stay under the Mozilla Public License 2.0
([LICENSE-MPL-2.0](https://github.com/pyrlyn/crates/blob/main/rust/d2-render/LICENSE-MPL-2.0)),
as stated at the top of each of those files. Both license texts ship in the
crate package.
