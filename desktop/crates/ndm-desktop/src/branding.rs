use gpui_kit::{AssetSource, IntoElement, Pixels, Result, SharedString, img, prelude::*};
use std::borrow::Cow;

pub(crate) const LOGO: &str = "branding/ndm-next.png";

pub(crate) struct Assets;
gpui_kit::assets::icon_assets!(ExtraIcons, [Zap]);

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == LOGO {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../../extension/icons/icon-128.png"
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
        if LOGO.starts_with(path) {
            paths.push(LOGO.into());
        }
        Ok(paths)
    }
}

pub(crate) fn logo(size: Pixels) -> impl IntoElement {
    img(LOGO).size(size).flex_shrink_0()
}
