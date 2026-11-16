# gpui-motion-kit 0.5.0

Source: published [gpui-motion-kit 0.5.0](https://crates.io/crates/gpui-motion-kit/0.5.0), from [1bye/gpui-motion](https://github.com/1bye/gpui-motion/tree/main/integrations/gpui-kit).
The package's VCS metadata records commit `ff948eb9e46d9a48343444bcb40d38c42e3255be`, path `integrations/gpui-kit`.

The 14 `src/` files are copied unchanged from the published crate. MIT and Apache-2.0 license texts are preserved alongside them. Examples and upstream test fixtures are omitted; this directory is excluded from the application workspace's tests and formatting.

Compatibility change: the upstream manifest pins GPUI Kit 0.7.3 (gpui-pre 0.3.9); this manifest instead pins Kit 0.10.0 (gpui-pre 0.3.12), matching the application. The portable engine is pinned to 0.4.0 with the upstream `text` and `svg-path` features. No CE renderer is enabled. The application tests exercise native frame sampling, focus, and buttons on the shared backend.

Replace this local dependency with the published adapter when upstream supports Kit 0.10. Do not independently edit the vendored animation implementation; keep application policy and styling in `ndm-desktop`.
