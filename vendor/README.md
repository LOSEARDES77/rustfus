# Vendored shadcn-egui

This directory vendors **shadcn-egui 0.7.0** and the three support crates used
by this application (`shadcn-egui-core`, `shadcn-egui-font-inter`, and
`shadcn-egui-icons-lucide`). The sources were taken from upstream commit
`6e62a930437f5e7eb0532fc6f238c7c3877cefd4` in the `oetiker/shadcn-egui`
repository (the commit recorded by each crate's `.cargo_vcs_info.json`).

The port keeps the upstream components and assets intact. The supplied
vendored sources already target egui **0.36.2** and egui_extras **0.36.2**;
no component API substitutions were needed. The normalized manifests use
relative paths for core, Inter, and Lucide. The repository root excludes
`vendor/*` from its workspace and patches the four published shadcn package
identities to these local copies. The shadcn crate retains its isolated
`[workspace]` for standalone checks and patches core there too, so optional
published font/icon crates share the same core types in standalone builds.
All upstream optional published dependencies and feature definitions remain.

Validation: `cargo check -p rustfus-gui` and
`cargo check --manifest-path vendor/shadcn-egui/Cargo.toml --all-features`.

Upstream license and font/icon license files are preserved in each crate
directory. This README documents only the integration port; component
implementations are not reimplemented here.
