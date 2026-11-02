use std::borrow::Cow;

use gpui_kit::component::Icon;
use gpui_kit::{AssetSource, Result, SharedString};

#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

const ICONS: &[(&str, &[u8])] = &[
    (
        "icons/ban.svg",
        include_bytes!("../../../res/icons/ban.svg"),
    ),
    (
        "icons/check.svg",
        include_bytes!("../../../res/icons/check.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../../../res/icons/chevron-down.svg"),
    ),
    (
        "icons/chevron-left.svg",
        include_bytes!("../../../res/icons/chevron-left.svg"),
    ),
    (
        "icons/chevron-right.svg",
        include_bytes!("../../../res/icons/chevron-right.svg"),
    ),
    (
        "icons/chevron-up.svg",
        include_bytes!("../../../res/icons/chevron-up.svg"),
    ),
    (
        "icons/chevrons-up-down.svg",
        include_bytes!("../../../res/icons/chevrons-up-down.svg"),
    ),
    (
        "icons/circle.svg",
        include_bytes!("../../../res/icons/circle.svg"),
    ),
    (
        "icons/circle-alert.svg",
        include_bytes!("../../../res/icons/circle-alert.svg"),
    ),
    (
        "icons/circle-check.svg",
        include_bytes!("../../../res/icons/circle-check.svg"),
    ),
    (
        "icons/circle-x.svg",
        include_bytes!("../../../res/icons/circle-x.svg"),
    ),
    (
        "icons/close.svg",
        include_bytes!("../../../res/icons/close.svg"),
    ),
    (
        "icons/copy.svg",
        include_bytes!("../../../res/icons/copy.svg"),
    ),
    (
        "icons/download.svg",
        include_bytes!("../../../res/icons/download.svg"),
    ),
    (
        "icons/ellipsis.svg",
        include_bytes!("../../../res/icons/ellipsis.svg"),
    ),
    (
        "icons/external-link.svg",
        include_bytes!("../../../res/icons/external-link.svg"),
    ),
    (
        "icons/eye.svg",
        include_bytes!("../../../res/icons/eye.svg"),
    ),
    (
        "icons/eye-off.svg",
        include_bytes!("../../../res/icons/eye-off.svg"),
    ),
    (
        "icons/file.svg",
        include_bytes!("../../../res/icons/file.svg"),
    ),
    (
        "icons/file-image.svg",
        include_bytes!("../../../res/icons/file-image.svg"),
    ),
    (
        "icons/folder-open.svg",
        include_bytes!("../../../res/icons/folder-open.svg"),
    ),
    (
        "icons/hard-drive.svg",
        include_bytes!("../../../res/icons/hard-drive.svg"),
    ),
    (
        "icons/inbox.svg",
        include_bytes!("../../../res/icons/inbox.svg"),
    ),
    (
        "icons/info.svg",
        include_bytes!("../../../res/icons/info.svg"),
    ),
    (
        "icons/loader.svg",
        include_bytes!("../../../res/icons/loader.svg"),
    ),
    (
        "icons/loader-circle.svg",
        include_bytes!("../../../res/icons/loader-circle.svg"),
    ),
    (
        "icons/minus.svg",
        include_bytes!("../../../res/icons/minus.svg"),
    ),
    (
        "icons/moon.svg",
        include_bytes!("../../../res/icons/moon.svg"),
    ),
    (
        "icons/plus.svg",
        include_bytes!("../../../res/icons/plus.svg"),
    ),
    (
        "icons/refresh-cw.svg",
        include_bytes!("../../../res/icons/refresh-cw.svg"),
    ),
    (
        "icons/search.svg",
        include_bytes!("../../../res/icons/search.svg"),
    ),
    (
        "icons/settings.svg",
        include_bytes!("../../../res/icons/settings.svg"),
    ),
    (
        "icons/sun.svg",
        include_bytes!("../../../res/icons/sun.svg"),
    ),
    (
        "icons/triangle-alert.svg",
        include_bytes!("../../../res/icons/triangle-alert.svg"),
    ),
    (
        "icons/usb.svg",
        include_bytes!("../../../res/icons/usb.svg"),
    ),
    // Kit's platform title bar paths use these Lucide equivalents.
    (
        "icons/window-close.svg",
        include_bytes!("../../../res/icons/window-close.svg"),
    ),
    (
        "icons/window-maximize.svg",
        include_bytes!("../../../res/icons/window-maximize.svg"),
    ),
    (
        "icons/window-minimize.svg",
        include_bytes!("../../../res/icons/window-minimize.svg"),
    ),
    (
        "icons/window-restore.svg",
        include_bytes!("../../../res/icons/window-restore.svg"),
    ),
    ("icons/x.svg", include_bytes!("../../../res/icons/x.svg")),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| (*name).into())
            .collect())
    }
}

/// Use a selected basename such as `usb`, without the directory or `.svg` suffix.
pub fn icon(name: &str) -> Icon {
    Icon::default().path(format!("icons/{name}.svg"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::{IconName, IconNamed};
    use std::collections::HashSet;

    #[test]
    fn selected_assets_are_unique_svg_documents() {
        assert_eq!(ICONS.len(), 40);
        assert!(ICONS.len() <= 40);
        let mut paths = HashSet::new();
        for &(path, bytes) in ICONS {
            assert!(paths.insert(path), "duplicate asset: {path}");
            assert!(path.starts_with("icons/") && path.ends_with(".svg"));
            let svg = std::str::from_utf8(bytes).expect("SVG is UTF-8");
            assert!(svg.trim_start().starts_with("<svg"), "{path}");
            assert!(
                svg.contains("xmlns=\"http://www.w3.org/2000/svg\""),
                "{path}"
            );
            assert!(svg.trim_end().ends_with("</svg>"), "{path}");
            assert!(matches!(Assets.load(path).unwrap(), Some(Cow::Borrowed(_))));
        }
        assert_eq!(Assets.list("icons/").unwrap().len(), ICONS.len());
        assert!(Assets.load("icons/not-bundled.svg").unwrap().is_none());
        assert!(Assets.list("fonts/").unwrap().is_empty());
    }

    #[test]
    fn shell_component_defaults_resolve() {
        // Select/list, checkbox, input, popup menu, alert/notification,
        // spinner and native title-bar defaults used by the shell.
        for name in [
            IconName::ChevronDown,
            IconName::ChevronUp,
            IconName::ChevronLeft,
            IconName::ChevronRight,
            IconName::ChevronsUpDown,
            IconName::Check,
            IconName::Close,
            IconName::Inbox,
            IconName::Search,
            IconName::Loader,
            IconName::Info,
            IconName::CircleCheck,
            IconName::CircleX,
            IconName::TriangleAlert,
            IconName::ExternalLink,
            IconName::Eye,
            IconName::EyeOff,
            IconName::Plus,
            IconName::Minus,
            IconName::WindowClose,
            IconName::WindowMinimize,
            IconName::WindowMaximize,
            IconName::WindowRestore,
        ] {
            let path = name.path();
            assert!(Assets.load(path.as_ref()).unwrap().is_some(), "{path}");
        }
    }
}
