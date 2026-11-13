//! Ícones embutidos no binário.
//!
//! O `Assets` padrão do gpui-kit só traz os ~100 ícones dos componentes. Os
//! ícones Lucide extras que o Cofre usa precisam ser listados aqui — senão
//! aparecem como um espaço vazio (sem erro). O teste abaixo confere todos.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        CreditCard,
        IdCard,
        KeyRound,
        LayoutGrid,
        Lock,
        LockKeyhole,
        Monitor,
        StickyNote,
        Trash
    ]
);

/// Glifo da marca (cadeado de chapéu), pintado com a cor do tema.
pub const BRAND_MARK: &str = "brand/mark.svg";
const BRAND_MARK_SVG: &[u8] = include_bytes!("../assets/brand/mark.svg");

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == BRAND_MARK {
            return Ok(Some(Cow::Borrowed(BRAND_MARK_SVG)));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        if BRAND_MARK.starts_with(path) {
            paths.push(BRAND_MARK.into());
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::assets::IconName;
    use gpui_kit::AssetSource;

    use super::AppAssets;

    /// Todos os ícones que o código usa. Ao usar um novo, acrescente aqui.
    const USED: &[IconName] = &[
        IconName::LockKeyhole,
        IconName::Lock,
        IconName::LayoutGrid,
        IconName::Star,
        IconName::StarFill,
        IconName::Globe,
        IconName::CreditCard,
        IconName::IdCard,
        IconName::StickyNote,
        IconName::KeyRound,
        IconName::FileText,
        IconName::Building2,
        IconName::Folder,
        IconName::FolderOpen,
        IconName::Trash,
        IconName::Search,
        IconName::RefreshCw,
        IconName::Copy,
        IconName::Eye,
        IconName::EyeOff,
        IconName::Settings,
        IconName::Monitor,
        IconName::Sun,
        IconName::Moon,
        IconName::Close,
    ];

    #[test]
    fn default_bundle_lacks_extras() {
        // Garante que o teste abaixo não passa à toa: o padrão não tem estes.
        let path = IconName::LockKeyhole.path();
        // `load` devolve erro (não `None`) para ícone ausente.
        assert!(!matches!(gpui_kit::assets::Assets.load(&path), Ok(Some(_))));
    }

    #[test]
    fn brand_mark_is_embedded() {
        let svg = AppAssets.load(super::BRAND_MARK).unwrap().unwrap();
        assert!(std::str::from_utf8(&svg).unwrap().starts_with("<svg"));
    }

    #[test]
    fn every_used_icon_is_embedded() {
        for icon in USED {
            let path = icon.path();
            assert!(
                AppAssets.load(&path).unwrap().is_some(),
                "ícone não embutido: {path} — acrescente em ExtraIcons"
            );
        }
    }
}
