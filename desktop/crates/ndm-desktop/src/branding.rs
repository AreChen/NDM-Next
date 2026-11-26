use gpui_kit::{AssetSource, IntoElement, Pixels, Result, SharedString, img, prelude::*};
use std::borrow::Cow;

pub(crate) const LIGHT_LOGO: &str = "branding/onesdm-light.png";
pub(crate) const LOGO: &str = "branding/ndm-next.png";

pub(crate) struct Assets;
gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        Zap,
        Clock,
        Trash,
        Store,
        Film,
        Puzzle,
        SearchX,
        PackageOpen,
        ArrowDownToLine,
        Sparkles,
        Terminal
    ]
);

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == LOGO {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../../extension/icons/icon-128.png"
            ))));
        }
        if path == LIGHT_LOGO {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../../extension/icons/icon-light-128.png"
            ))));
        }
        if let Some(icon) = ExtraIcons.load(path)? {
            return Ok(Some(icon));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.extend(
            [LOGO, LIGHT_LOGO]
                .into_iter()
                .filter(|logo| logo.starts_with(path))
                .map(SharedString::from),
        );
        Ok(paths)
    }
}

pub(crate) fn logo(size: Pixels, tone: crate::motion::ThemeTone) -> impl IntoElement {
    img(if tone.darkness >= 0.5 {
        LOGO
    } else {
        LIGHT_LOGO
    })
    .size(size)
    .flex_shrink_0()
}
