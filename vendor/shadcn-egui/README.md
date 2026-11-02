# shadcn-egui

[![CI](https://github.com/oetiker/shadcn-egui/actions/workflows/ci.yml/badge.svg)](https://github.com/oetiker/shadcn-egui/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/oetiker/shadcn-egui)](https://github.com/oetiker/shadcn-egui/releases/latest)
[![crates.io](https://img.shields.io/crates/v/shadcn-egui)](https://crates.io/crates/shadcn-egui)
[![docs.rs](https://img.shields.io/docsrs/shadcn-egui)](https://docs.rs/shadcn-egui)

The [shadcn/ui](https://ui.shadcn.com/) design system for
[egui](https://github.com/emilk/egui): the shadcn v4 styles (vega, nova,
maia, lyra, mira, luma, rhea), their colour themes, fonts, icon sets and
presets, and components drawn as plain egui widgets. It works with any egui
backend (eframe, egui-winit, glow, wgpu).

![shadcn-egui component gallery](https://raw.githubusercontent.com/oetiker/shadcn-egui/v0.7.0/docs/gallery-screen.png)

[Try the gallery in your browser](https://oetiker.github.io/shadcn-egui/):
every component under the preset pickers, with all fonts and icon sets. The
first load downloads about 18 MB.

## Install

```bash
cargo add shadcn-egui
```

## Use a shadcn preset

Build a look on the shadcn create page (ui.shadcn.com), copy its preset code
and apply it. Codes from shadcn-svelte.com number colours differently and
are not supported.

```rust
use shadcn_egui::Preset;

fn setup(ctx: &egui::Context) -> Result<(), shadcn_egui::PresetCodeError> {
    Preset::from_code("b2D0wqNxT")?.apply(ctx);
    ctx.set_theme(egui::Theme::Dark);
    Ok(())
}
```

This preset uses the Geist font and the Hugeicons icons, which need
`--features font-geist,icons-hugeicons`; without them it falls back to
Inter and Lucide. Its translucent menu colour always falls back to the
opaque one, with a log warning.

The theme holds the light and the dark palette: `ctx.set_theme(..)` switches
the mode, and `apply` again only for another preset; the fonts are
reinstalled only when they change. An app that installs fonts of its own uses
`Theme::apply_with_fonts` instead (see the `Theme` docs). Without a preset,
`Theme::vega().apply(ctx)` gives the default look.

## Components

Button, Badge, Input, Label, Card, Tabs, Switch, Checkbox, Select, Slider,
Separator, Tooltip, Popover, Dialog and Icon (`IconView`):

```rust
use shadcn_egui::components::button::Button;

fn ui(ui: &mut egui::Ui) {
    if ui.add(Button::new("Save changes")).clicked() {
        // ...
    }
}
```

In a git checkout of this repository,
`cargo run --example gallery --features fonts-all,icons-all` shows every
component, each variant and size, and the icons of the chosen icon library.
Paste a preset code or pick each preset part (style, colours, fonts, icon
library, radius, menu) and the light or dark mode, and the gallery restyles
live.

`cargo run --example settings` opens a settings page built from Badge,
Button, Card, Checkbox, Input, Label, Select, Separator, Slider, Switch and
Tabs, with a toolbar that switches style, mode and radius. The image shows
it at a desktop width (1280 px) and at a phone width (390 px).

![shadcn-egui settings screen at 1280 px and at 390 px wide](https://raw.githubusercontent.com/oetiker/shadcn-egui/v0.7.0/docs/settings-screen.png)

## Features

Inter and the Lucide icons are always included. Everything else is a Cargo
feature named after the shadcn id:

| Feature | Adds |
|---|---|
| `font-<id>` | one font of the create page, for example `font-geist`, `font-jetbrains-mono`, `font-playfair-display` |
| `fonts-all` | every font |
| `icons-tabler` | Tabler icons |
| `icons-phosphor` | Phosphor icons |
| `icons-remixicon` | Remix Icon |
| `icons-hugeicons` | Hugeicons |
| `icons-all` | every icon set |

A preset that names a font or icon set whose feature is off renders with
Inter or Lucide and logs a warning through the `log` crate. The `sera`
style has no egui version yet and falls back to `nova`.

## Licences

The code is MIT (`LICENSE`). Each font crate also ships its font's licence
(SIL Open Font License 1.1 or Apache License 2.0). The icon crates ship
theirs: Lucide ISC, Tabler and Phosphor MIT, Remix Icon Apache-2.0 (version
4.8.0, the last release under that licence; later releases are not used),
Hugeicons MIT. Each crate's `license` field names both the code and the
bundled licence.

## The Claude Code plugin

This repository also hosts the `shadcn-egui` Claude Code plugin. Its skill
`build-ui` builds or restyles an egui UI with this crate, and
`port-component` ports a shadcn component the crate does not have yet, into
your app or into this repository.

```text
/plugin marketplace add oetiker/shadcn-egui
/plugin install shadcn-egui@shadcn-egui
```

An install of the older `egui-shadcn` plugin is removed first:

```text
/plugin uninstall egui-shadcn@egui-shadcn-dev
/plugin marketplace remove egui-shadcn-dev
```

## Development

See [CLAUDE.md](CLAUDE.md) for the layout, generators and checks, and
[RELEASING.md](RELEASING.md) for releases.
